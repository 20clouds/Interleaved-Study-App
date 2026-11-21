// Settings: defaults, deep merge with what is stored, and the safety check on provider addresses.
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// AI providers. `routine` is a Claude scheduled task fired over its API trigger and paid for by a Claude subscription.
pub const PROVIDERS: [&str; 4] = ["gemini", "anthropic", "openai", "routine"];
/// Everything a secret can be stored for: the providers plus the GitHub token the scheduled-task mailbox may use.
pub const SECRETS: [&str; 5] = ["gemini", "anthropic", "openai", "routine", "github"];

pub fn provider_label(p: &str) -> &'static str {
    match p {
        "anthropic" => "Anthropic Claude",
        "openai" => "OpenAI",
        "routine" => "Claude scheduled task",
        _ => "Google Gemini",
    }
}

pub fn key_name(p: &str) -> &'static str {
    match p {
        "anthropic" => "Claude API key",
        "openai" => "OpenAI API key",
        "routine" => "scheduled task token",
        _ => "Google AI Studio API key",
    }
}

pub fn default_base_url(p: &str) -> &'static str {
    match p {
        "anthropic" => "https://api.anthropic.com",
        "openai" => "https://api.openai.com",
        "gemini" => "https://generativelanguage.googleapis.com",
        _ => "",
    }
}

pub const GEMINI_DEFAULT_MODEL: &str = "gemini-3.5-flash-lite";
pub const ROUTINE_MODEL: &str = "claude-scheduled-task";
const DEFAULTS_REV: i64 = 3;

pub fn default_settings() -> Value {
    json!({
        "ai": {
            "provider": "gemini",
            "model": GEMINI_DEFAULT_MODEL,
            "lastModels": {},
            "endpoint": "auto",
            "baseUrls": { "gemini": default_base_url("gemini"), "anthropic": default_base_url("anthropic"), "openai": default_base_url("openai") },
            "includeModelInCacheKey": false,
            "windowTokens": 3500,
            "maxOutputTokens": 1200,
            "minimalThinking": true,
            "thinking": {},
            "defaultsRev": DEFAULTS_REV,
            "routine": { "fireUrl": "", "repo": "", "batchMax": 8, "batchChars": 200000, "gapMinutes": 0, "timeoutMinutes": 30, "windowTokens": 6000, "githubApi": "" }
        },
        "study": { "minutes": 30, "useSuggested": false, "pace": 120 },
        "pomo": { "work": 25, "brk": 5, "long": 15, "cycles": 4, "perTopic": "one", "n": 3, "minutes": 40, "autoStartNext": false },
        "notify": { "sound": "Glass", "bounce": true, "badge": true },
        "goal": { "kind": "minutes", "value": 60, "restDaysPerWeek": 1 },
        "theme": "system"
    })
}

/// Deep-merge `patch` over `base`. Keys missing in patch keep base values, so new defaults survive old data.
pub fn deep_merge(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            let mut out: Map<String, Value> = b.clone();
            for (k, v) in p {
                let merged = match b.get(k) {
                    Some(bv) if bv.is_object() && v.is_object() => deep_merge(bv, v),
                    _ => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        _ => patch.clone(),
    }
}

/// Why a base URL can't receive an API key, or None when it can (blank means the official endpoint).
/// https only (http just for this machine), and no user name, query or fragment that could hide the real host.
pub fn base_url_problem(u: &str) -> Option<&'static str> {
    let t = u.trim();
    if t.is_empty() { return None }
    let Ok(url) = url::Url::parse(t) else { return Some("is not a valid web address") };
    let host = url.host_str().unwrap_or("");
    let local = host == "localhost" || host == "127.0.0.1" || host == "[::1]";
    if url.scheme() != "https" && !(url.scheme() == "http" && local) { return Some("must start with https:// (http:// only for localhost)") }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Some("cannot contain a user name, password, ?query or #fragment");
    }
    None
}

pub fn url_host(u: &str) -> String {
    url::Url::parse(u.trim()).ok().map(|x| match x.port() { Some(p) => format!("{}:{p}", x.host_str().unwrap_or("")), None => x.host_str().unwrap_or("").to_string() }).unwrap_or_default()
}

/// Stored settings merged over the defaults, with the one-time migrations applied.
pub fn merge_settings(stored: Option<&Value>) -> Value {
    let empty = json!({});
    let stored = stored.unwrap_or(&empty);
    let mut out = deep_merge(&default_settings(), stored);
    let ai = out.get_mut("ai").and_then(|x| x.as_object_mut()).expect("ai settings");
    // Settings from before multi-provider support had a single Gemini `baseUrl`.
    if let Some(Value::String(legacy)) = ai.remove("baseUrl") {
        let had = stored.pointer("/ai/baseUrls/gemini").is_some();
        if !had && !legacy.is_empty() { ai["baseUrls"]["gemini"] = Value::String(legacy); }
    }
    // Stored or restored addresses that fail the safety check fall back to the official endpoint.
    if let Some(urls) = ai.get_mut("baseUrls").and_then(|x| x.as_object_mut()) {
        for (p, u) in urls.iter_mut() {
            let bad = match u.as_str() { Some(s) => base_url_problem(s).is_some(), None => true };
            if bad { *u = Value::String(default_base_url(p).to_string()); }
        }
    }
    // Revision 2 made Flash-Lite the Gemini default: settings saved before it move to it once.
    let rev = stored.pointer("/ai/defaultsRev").and_then(|x| x.as_i64()).unwrap_or(0);
    if rev < 2 {
        if ai.get("provider").and_then(|x| x.as_str()) == Some("gemini") { ai.insert("model".into(), json!(GEMINI_DEFAULT_MODEL)); }
        if let Some(m) = ai.get_mut("lastModels").and_then(|x| x.as_object_mut()) { m.insert("gemini".into(), json!(GEMINI_DEFAULT_MODEL)); }
    }
    // Revision 3 sized a scheduled-task run after the first real ones: eight 6,000-token windows fit one run, where
    // the earlier 9,000-token windows fit only three under the size cap. Earlier values move to it once.
    if rev < 3 {
        let fresh = default_settings();
        if let Some(r) = ai.get_mut("routine").and_then(|x| x.as_object_mut()) {
            for k in ["batchMax", "batchChars", "windowTokens"] { r.insert(k.into(), fresh["ai"]["routine"][k].clone()); }
        }
    }
    ai.insert("defaultsRev".into(), json!(DEFAULTS_REV));
    for k in ["rpm", "tpm", "rpd", "utilization", "countInputOnly"] { ai.remove(k); }
    if !PROVIDERS.contains(&ai.get("provider").and_then(|x| x.as_str()).unwrap_or("")) { ai.insert("provider".into(), json!("gemini")); }
    // A path to an external converter, never used: settings and backups from before carry it, and it is dropped.
    if let Some(o) = out.as_object_mut() { o.remove("tools"); }
    out
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RoutineSettings {
    pub fire_url: String,
    pub repo: String,
    pub batch_max: f64,
    pub batch_chars: f64,
    pub gap_minutes: f64,
    pub timeout_minutes: f64,
    pub window_tokens: f64,
    pub github_api: String,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub base_urls: HashMap<String, String>,
    pub include_model_in_cache_key: bool,
    pub window_tokens: f64,
    pub max_output_tokens: f64,
    pub minimal_thinking: bool,
    #[serde(default)]
    pub thinking: HashMap<String, String>,
    pub routine: RoutineSettings,
}

impl AiSettings {
    /// The base URL for a provider; a blank or unsafe override falls back to the official endpoint.
    pub fn base_url(&self, p: &str) -> String {
        match self.base_urls.get(p).map(|u| u.trim()) {
            Some(u) if !u.is_empty() && base_url_problem(u).is_none() => u.to_string(),
            _ => default_base_url(p).to_string(),
        }
    }

    pub fn is_routine(&self) -> bool {
        self.provider == "routine"
    }

    /// Paragraph window size: a cloud session reads far more at once than a small API request should carry.
    pub fn window(&self) -> f64 {
        if self.is_routine() { self.routine.window_tokens.max(1000.0) } else { self.window_tokens.max(500.0) }
    }

    /// The thinking level to send to a Gemini model: the one chosen for it while the model accepts it, else its fastest.
    pub fn gemini_level(&self) -> String {
        let levels = gemini_levels(&self.model);
        match self.thinking.get(&self.model) {
            Some(l) if levels.contains(&l.as_str()) => l.clone(),
            _ => levels[0].to_string(),
        }
    }
}

/// The thinking levels each Gemini model accepts, fastest first (Gemini API thinking docs, checked 2026-11-02).
pub fn gemini_levels(model: &str) -> Vec<&'static str> {
    const ALL: [&str; 4] = ["minimal", "low", "medium", "high"];
    const NO_MIN: [&str; 3] = ["low", "medium", "high"];
    match model {
        "gemini-3.5-flash-lite" | "gemini-3.6-flash" | "gemini-3-flash-preview" => ALL.to_vec(),
        "gemini-3.8-flash" | "gemini-flash-latest" | "gemini-4-argon" | "gemini-3.1-pro-preview" | "gemini-2.5-pro" | "gemini-2.5-flash" | "gemini-2.5-flash-lite" => NO_MIN.to_vec(),
        "gemini-3.1-flash-lite-image" => vec!["minimal", "high"],
        "gemini-3-pro-preview" => vec!["low", "high"],
        m if m.to_lowercase().starts_with("gemma-4") => vec!["minimal", "high"],
        _ => ALL.to_vec(),
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StudySettings { pub minutes: f64, pub use_suggested: bool, pub pace: f64 }

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PomoSettings { pub work: f64, pub brk: f64, pub long: f64, pub cycles: f64, pub per_topic: String, pub n: f64, pub minutes: f64, pub auto_start_next: bool }

#[derive(Deserialize, Clone, Debug)]
pub struct NotifySettings { pub sound: String, pub bounce: bool, pub badge: bool }

#[derive(Deserialize, Clone, Debug)]
pub struct Settings {
    pub ai: AiSettings,
    pub study: StudySettings,
    pub pomo: PomoSettings,
    pub notify: NotifySettings,
    pub theme: String,
}

/// The typed view of merged settings. Values of the wrong type (a damaged file) fall back to the defaults.
pub fn typed(v: &Value) -> Settings {
    serde_json::from_value(v.clone()).unwrap_or_else(|_| serde_json::from_value(default_settings()).expect("default settings are valid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_defaults_and_choices() {
        let s = merge_settings(Some(&json!({ "ai": { "model": "x", "defaultsRev": 2 }, "study": { "minutes": 45 } })));
        assert_eq!(s["ai"]["model"], "x");
        assert_eq!(s["study"]["minutes"], 45);
        assert_eq!(s["study"]["pace"], 120);
        assert_eq!(typed(&s).ai.routine.batch_max, 8.0);
        let kept = merge_settings(Some(&json!({ "ai": { "defaultsRev": 3, "routine": { "batchMax": 4 } } })));
        assert_eq!(typed(&kept).ai.routine.batch_max, 4.0);
    }

    #[test]
    fn old_settings_move_to_flash_lite_once() {
        let s = merge_settings(Some(&json!({ "ai": { "model": "gemini-3.8-flash" } })));
        assert_eq!(s["ai"]["model"], GEMINI_DEFAULT_MODEL);
    }

    #[test]
    fn base_urls_are_checked() {
        assert!(base_url_problem("http://example.com").is_some());
        assert!(base_url_problem("https://user@example.com").is_some());
        assert!(base_url_problem("http://localhost:8787").is_none());
        assert!(base_url_problem("").is_none());
        let s = merge_settings(Some(&json!({ "ai": { "baseUrls": { "openai": "http://evil.example" }, "defaultsRev": 2 } })));
        assert_eq!(s["ai"]["baseUrls"]["openai"], "https://api.openai.com");
    }
}
