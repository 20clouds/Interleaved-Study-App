// Provider adapters over plain HTTPS: Google AI (generateContent and Interactions), Anthropic Claude (Messages)
// and OpenAI (Responses). What a model accepts (system instruction, JSON mode, structured output, minimal
// thinking / low effort, Claude's refusal fallback) is learned per model: on HTTP 400 the offending feature is
// dropped and the request sent again without it.
use crate::aijson::extract_json;
use crate::files::Attachment;
use base64::Engine;
use crate::rx;
use crate::types::{ModelCaps, TestResult};
use crate::util::{now_ms, take_chars};
use serde_json::{json, Map, Value};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const MAX_ATTEMPTS: i64 = 6;
/// Longest server-requested wait honored by a retry; longer ones pause the queue instead.
pub const MAX_RETRY_AFTER_MS: f64 = 3_600_000.0;
/// Longest wait reported to the person: a daily free-tier quota can ask for most of a day.
pub const MAX_RESET_HINT_MS: f64 = 48.0 * 3_600_000.0;
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(90);
/// A request carrying a file takes longer to upload and to read.
const FILE_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone)]
pub enum AiErr {
    Http { status: u16, body: String, retry_after: Option<f64> },
    Network(String),
    Blocked(String),
    Cancelled,
    Other(String),
}

impl AiErr {
    pub fn message(&self) -> String {
        match self {
            AiErr::Http { status, body, .. } => format!("HTTP {status}: {}", summarize_body(body)),
            AiErr::Network(m) | AiErr::Blocked(m) | AiErr::Other(m) => m.clone(),
            AiErr::Cancelled => "Cancelled".into(),
        }
    }

    /// A 400 that is about the key, not a feature (Gemini answers a bad key with 400 "API key not valid").
    pub fn is_auth_400(&self) -> bool {
        matches!(self, AiErr::Http { status: 400, body, .. } if rx!(r"(?i)api[ _-]?key|authenticat|credential").is_match(body))
    }

    pub fn is_key_problem(&self) -> bool {
        matches!(self, AiErr::Http { status: 401 | 403, .. }) || self.is_auth_400()
    }
}

fn summarize_body(body: &str) -> String {
    if let Ok(j) = serde_json::from_str::<Value>(body) {
        let e = &j["error"];
        if let Some(m) = e["message"].as_str() {
            let kind = e["status"].as_str().or_else(|| e["type"].as_str());
            return take_chars(&match kind { Some(k) => format!("{k}: {m}"), None => m.to_string() }, 400);
        }
    }
    take_chars(body, 400)
}

pub fn backoff_ms(attempt: i64, retry_after: Option<f64>) -> f64 {
    let base = (2000.0 * 2f64.powi((attempt - 1).max(0) as i32)).min(120_000.0);
    let jittered = (base * (0.5 + 0.5 * rand::random::<f64>())).round();
    match retry_after { Some(r) if r > 0.0 => r.min(MAX_RETRY_AFTER_MS).max(jittered), _ => jittered }
}

/// Parse a Retry-After header (seconds or HTTP date), a google.rpc.RetryInfo delay like "17s", or Google's
/// "Please retry in 8h23m14.1s", capped at MAX_RESET_HINT_MS.
pub fn parse_retry_after(header: Option<&str>, body: &str, now: f64) -> Option<f64> {
    let cap = |ms: f64| ms.clamp(0.0, MAX_RESET_HINT_MS);
    if let Some(h) = header.map(|h| h.trim()).filter(|h| !h.is_empty()) {
        if let Ok(n) = h.parse::<f64>() { if n.is_finite() { return Some(cap(n * 1000.0)) } }
        if let Ok(t) = jiff::fmt::rfc2822::parse(h) { return Some(cap(t.timestamp().as_millisecond() as f64 - now)) }
    }
    if let Some(m) = rx!(r#""retryDelay"\s*:\s*"(\d+(?:\.\d+)?)s""#).captures(body) {
        return m[1].parse::<f64>().ok().map(|s| cap((s * 1000.0).round()));
    }
    if let Some(t) = rx!(r"(?i)retry in (?:(\d{1,3})h)?(?:(\d{1,4})m)?(?:(\d{1,6}(?:\.\d{1,12})?)s)?").captures(body) {
        if t.get(1).is_some() || t.get(2).is_some() || t.get(3).is_some() {
            let n = |i: usize| t.get(i).and_then(|x| x.as_str().parse::<f64>().ok()).unwrap_or(0.0);
            return Some(cap((((n(1) * 60.0 + n(2)) * 60.0 + n(3)) * 1000.0).round()));
        }
    }
    None
}

pub fn http_client() -> reqwest::Client {
    // No redirects: a base URL host cannot bounce the key header to another host.
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(20)).build().expect("http client")
}

pub struct HttpReply {
    pub status: u16,
    pub retry_after: Option<String>,
    pub text: String,
}

/// One request with a deadline that also covers reading the body, a size cap, and instant cancellation.
pub async fn send(req: reqwest::RequestBuilder, timeout: Duration, cancel: Option<&CancellationToken>) -> Result<HttpReply, AiErr> {
    let work = async {
        let mut res = req.send().await.map_err(|e| AiErr::Network(format!("Network error: {}", net_msg(&e))))?;
        let status = res.status().as_u16();
        let retry_after = res.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        let too_large = || AiErr::Other(format!("The response was larger than {} MB, so it was not read.", MAX_RESPONSE_BYTES / 1_048_576));
        if res.content_length().is_some_and(|n| n as usize > MAX_RESPONSE_BYTES) { return Err(too_large()) }
        let mut buf: Vec<u8> = vec![];
        loop {
            match res.chunk().await {
                Ok(Some(c)) => {
                    if buf.len() + c.len() > MAX_RESPONSE_BYTES { return Err(too_large()) }
                    buf.extend_from_slice(&c);
                }
                Ok(None) => break,
                // An error status whose body could not be read still reports its status.
                Err(e) => if (200..300).contains(&status) { return Err(AiErr::Network(format!("Network error: {}", net_msg(&e)))) } else { break },
            }
        }
        Ok(HttpReply { status, retry_after, text: String::from_utf8_lossy(&buf).to_string() })
    };
    let timed = async {
        match tokio::time::timeout(timeout, work).await {
            Ok(r) => r,
            Err(_) => Err(AiErr::Network(format!("Request timed out after {}s", timeout.as_secs()))),
        }
    };
    match cancel {
        Some(c) => tokio::select! { _ = c.cancelled() => Err(AiErr::Cancelled), r = timed => r },
        None => timed.await,
    }
}

fn net_msg(e: &reqwest::Error) -> String {
    let mut s = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(x) = src { s = x.to_string(); src = x.source(); }
    take_chars(&s, 200)
}

async fn post(http: &reqwest::Client, url: &str, headers: &[(&str, &str)], body: &Value, cancel: Option<&CancellationToken>) -> Result<Value, AiErr> {
    post_t(http, url, headers, body, cancel, TIMEOUT).await
}

async fn post_t(http: &reqwest::Client, url: &str, headers: &[(&str, &str)], body: &Value, cancel: Option<&CancellationToken>, timeout: Duration) -> Result<Value, AiErr> {
    let mut req = http.post(url).header("Content-Type", "application/json").body(body.to_string());
    for (k, v) in headers { req = req.header(*k, *v); }
    let r = send(req, timeout, cancel).await?;
    if !(200..300).contains(&r.status) {
        let retry_after = parse_retry_after(r.retry_after.as_deref(), &r.text, now_ms());
        return Err(AiErr::Http { status: r.status, body: r.text, retry_after });
    }
    serde_json::from_str(&r.text).map_err(|_| AiErr::Http { status: 502, body: format!("Non-JSON response: {}", take_chars(&r.text, 200)), retry_after: None })
}

pub struct ProviderCfg<'a> {
    pub provider: &'a str,
    pub base_url: &'a str,
    pub key: &'a str,
    pub endpoint: &'a str, // auto | generateContent | interactions
}

pub struct CallOpts<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub prompt: &'a str,
    pub max_output: f64,
    pub json: bool,
    pub think_minimal: bool,
    pub level: Option<&'a str>, // Gemini: the level to ask for when think_minimal is on
    pub schema: Option<&'a Value>,
    pub cancel: Option<&'a CancellationToken>,
    /// The lesson's original file. Only the Gemini adapters send it.
    pub attachment: Option<&'a Attachment>,
}

pub struct CallResult {
    pub text: String,
    /// Whether the attachment was part of the request that answered.
    pub file_sent: bool,
}

pub fn default_caps(model: &str, provider: &str) -> ModelCaps {
    let api = match provider { "anthropic" => "messages", "openai" => "responses", "routine" => "routine", _ => "generateContent" };
    ModelCaps { model: model.into(), api: api.into(), ..Default::default() }
}

// think: minimal thinking (Gemini) / low effort (Claude, OpenAI). low: Gemini level low, chosen or when minimal is
// rejected. deep: Gemini level medium or high, as chosen. room: Gemini thinking headroom.
#[derive(Clone)]
struct Use { sys: bool, json: bool, think: bool, low: bool, deep: Option<&'static str>, fallback: bool, schema: bool, room: bool, file: bool }

// Reasoning models count thinking tokens toward the output cap, so leave room for them on top of the answer.
const THINK_HEADROOM_LOW: f64 = 4096.0;
const THINK_HEADROOM_DEFAULT: f64 = 12000.0;

fn gemini_level(u: &Use) -> Option<&'static str> {
    u.deep.or(if u.think { Some("minimal") } else if u.low { Some("low") } else { None })
}

fn gemini_max_output(o: &CallOpts, u: &Use) -> f64 {
    if !u.room || u.think { o.max_output } else { o.max_output + if u.low { THINK_HEADROOM_LOW } else { THINK_HEADROOM_DEFAULT } }
}

/// Gemini models without structured output (image, speech, live, embedding...).
pub fn gemini_supports_schema(model: &str) -> bool {
    !rx!(r"(?i)(image|banana|tts|live|native-audio|embedding|transcribe|omni|lyria|veo)").is_match(model)
}

/// Claude models that accept the server-side refusal fallback (`fallbacks: "default"`).
pub fn claude_supports_fallback(model: &str) -> bool {
    rx!(r"^claude-(fable-5-1|opus-5-5|opus-5|sonnet-5-5)$").is_match(model)
}

fn folded(o: &CallOpts, u: &Use) -> String {
    if u.sys { o.prompt.to_string() } else { format!("{}\n\n{}", o.system, o.prompt) }
}

fn trim_base(u: &str) -> &str { u.trim_end_matches('/') }

fn b64(a: &Attachment) -> String {
    base64::engine::general_purpose::STANDARD.encode(a.data.as_slice())
}

/// The file goes first: repeated windows of one lesson then share a prefix Gemini can cache.
fn file_of<'a>(o: &CallOpts<'a>, u: &Use) -> Option<&'a Attachment> {
    if u.file { o.attachment } else { None }
}

/// With a file, the time limit grows with the answer asked for: writing out a scan takes minutes, not seconds.
fn timeout_for(o: &CallOpts, u: &Use) -> Duration {
    if file_of(o, u).is_none() { return TIMEOUT }
    FILE_TIMEOUT.max(Duration::from_secs_f64((120.0 + o.max_output / 120.0).min(600.0)))
}

async fn call_generate_content(http: &reqwest::Client, cfg: &ProviderCfg<'_>, o: &CallOpts<'_>, u: &Use) -> Result<CallResult, AiErr> {
    let mut gc = Map::new();
    gc.insert("maxOutputTokens".into(), json!(gemini_max_output(o, u)));
    if u.json {
        gc.insert("responseMimeType".into(), json!("application/json"));
        if let (true, Some(s)) = (u.schema, o.schema) { gc.insert("responseJsonSchema".into(), s.clone()); }
    }
    if let Some(l) = gemini_level(u) { gc.insert("thinkingConfig".into(), json!({ "thinkingLevel": l })); }
    let mut parts = vec![];
    if let Some(a) = file_of(o, u) { parts.push(json!({ "inlineData": { "mimeType": a.mime, "data": b64(a) } })); }
    parts.push(json!({ "text": folded(o, u) }));
    let mut body = json!({ "contents": [{ "role": "user", "parts": parts }], "generationConfig": gc });
    if u.sys { body["systemInstruction"] = json!({ "parts": [{ "text": o.system }] }); }
    let model: String = url::form_urlencoded::byte_serialize(o.model.as_bytes()).collect();
    let url = format!("{}/v1beta/models/{}:generateContent", trim_base(cfg.base_url), model);
    let j = post_t(http, &url, &[("x-goog-api-key", cfg.key)], &body, o.cancel, timeout_for(o, u)).await?;
    let Some(cand) = j["candidates"].get(0) else {
        let why = j["promptFeedback"]["blockReason"].as_str().map(|b| format!(" (blocked: {b})")).unwrap_or_default();
        return Err(AiErr::Blocked(format!("No answer from the model{why}")));
    };
    let text = cand["content"]["parts"].as_array().map(|ps| ps.iter().filter(|p| p["thought"].as_bool() != Some(true)).filter_map(|p| p["text"].as_str()).collect::<String>()).unwrap_or_default();
    Ok(CallResult { text, file_sent: file_of(o, u).is_some() })
}

pub fn interaction_text(j: &Value) -> String {
    if let Some(s) = j["output_text"].as_str() { return s.to_string() }
    if let Some(steps) = j["steps"].as_array() {
        return steps.iter().filter(|s| s["type"] == "model_output").flat_map(|s| s["content"].as_array().cloned().unwrap_or_default())
            .filter(|c| c["type"] == "text").filter_map(|c| c["text"].as_str().map(|s| s.to_string())).collect();
    }
    if let Some(outs) = j["outputs"].as_array() {
        return outs.iter().filter(|x| x["type"].as_str().unwrap_or("text") == "text").filter_map(|x| x["text"].as_str()).collect();
    }
    String::new()
}

async fn call_interactions(http: &reqwest::Client, cfg: &ProviderCfg<'_>, o: &CallOpts<'_>, u: &Use) -> Result<CallResult, AiErr> {
    let mut gc = Map::new();
    gc.insert("max_output_tokens".into(), json!(gemini_max_output(o, u)));
    if let Some(l) = gemini_level(u) { gc.insert("thinking_level".into(), json!(l)); }
    let input = match file_of(o, u) {
        Some(a) => json!([
            { "type": if a.mime.starts_with("image/") { "image" } else { "document" }, "data": b64(a), "mime_type": a.mime },
            { "type": "text", "text": folded(o, u) }
        ]),
        None => json!(folded(o, u)),
    };
    let mut body = json!({ "model": o.model, "input": input, "generation_config": gc });
    if u.sys { body["system_instruction"] = json!(o.system); }
    if u.json {
        let mut f = json!({ "type": "text", "mime_type": "application/json" });
        if let (true, Some(s)) = (u.schema, o.schema) { f["schema"] = s.clone(); }
        body["response_format"] = f;
    }
    let url = format!("{}/v1beta/interactions", trim_base(cfg.base_url));
    let j = post_t(http, &url, &[("x-goog-api-key", cfg.key), ("Api-Revision", "2026-05-20")], &body, o.cancel, timeout_for(o, u)).await?;
    let text = interaction_text(&j);
    if text.is_empty() {
        let st = j["status"].as_str().map(|s| format!(" (status {s})")).unwrap_or_default();
        return Err(AiErr::Blocked(format!("Empty answer from the Interactions API{st}")));
    }
    Ok(CallResult { text, file_sent: file_of(o, u).is_some() })
}

async fn call_messages(http: &reqwest::Client, cfg: &ProviderCfg<'_>, o: &CallOpts<'_>, u: &Use) -> Result<CallResult, AiErr> {
    let mut body = json!({
        "model": o.model,
        "max_tokens": o.max_output + if u.think { THINK_HEADROOM_LOW } else { THINK_HEADROOM_DEFAULT },
        "messages": [{ "role": "user", "content": folded(o, u) }]
    });
    if u.sys { body["system"] = json!(o.system); }
    if u.think { body["output_config"] = json!({ "effort": "low" }); }
    let mut headers = vec![("x-api-key", cfg.key), ("anthropic-version", "2023-06-01")];
    if u.fallback {
        body["fallbacks"] = json!("default");
        headers.push(("anthropic-beta", "server-side-fallback-2026-07-01"));
    }
    let url = format!("{}/v1/messages", trim_base(cfg.base_url));
    let j = post(http, &url, &headers, &body, o.cancel).await?;
    let stop = j["stop_reason"].as_str();
    if stop == Some("refusal") {
        let cat = j["stop_details"]["category"].as_str().map(|c| format!(" ({c})")).unwrap_or_default();
        return Err(AiErr::Blocked(format!("Claude declined this request{cat}")));
    }
    // With a fallback, text before the last fallback block came from a model that then declined.
    let blocks = j["content"].as_array().cloned().unwrap_or_default();
    let from = blocks.iter().rposition(|b| b["type"] == "fallback").map(|i| i + 1).unwrap_or(0);
    let text: String = blocks[from..].iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
    if text.is_empty() {
        return Err(AiErr::Blocked(format!("Empty answer from Claude{}", stop.map(|s| format!(" (stop reason {s})")).unwrap_or_default())));
    }
    Ok(CallResult { text, file_sent: file_of(o, u).is_some() })
}

/// OpenAI Responses API. Nothing is stored on OpenAI's side (`store: false`).
async fn call_responses(http: &reqwest::Client, cfg: &ProviderCfg<'_>, o: &CallOpts<'_>, u: &Use) -> Result<CallResult, AiErr> {
    let mut body = json!({
        "model": o.model, "input": folded(o, u),
        "max_output_tokens": o.max_output + if u.think { THINK_HEADROOM_LOW } else { THINK_HEADROOM_DEFAULT },
        "store": false
    });
    if u.sys { body["instructions"] = json!(o.system); }
    if u.json { body["text"] = json!({ "format": { "type": "json_object" } }); }
    if u.think { body["reasoning"] = json!({ "effort": "low" }); }
    let url = format!("{}/v1/responses", trim_base(cfg.base_url));
    let auth = format!("Bearer {}", cfg.key);
    let j = post(http, &url, &[("Authorization", auth.as_str())], &body, o.cancel).await?;
    let parts: Vec<Value> = j["output"].as_array().map(|a| a.iter().filter(|x| x["type"] == "message").flat_map(|x| x["content"].as_array().cloned().unwrap_or_default()).collect()).unwrap_or_default();
    if let Some(r) = parts.iter().find(|c| c["type"] == "refusal") {
        let why = r["refusal"].as_str().map(|s| format!(": {}", take_chars(s, 200))).unwrap_or_default();
        return Err(AiErr::Blocked(format!("The model declined this request{why}")));
    }
    let text: String = parts.iter().filter(|c| c["type"] == "output_text").filter_map(|c| c["text"].as_str()).collect();
    if text.is_empty() {
        let why = j["incomplete_details"]["reason"].as_str().map(|r| format!(" (incomplete: {r})")).or_else(|| j["status"].as_str().map(|s| format!(" (status {s})"))).unwrap_or_default();
        return Err(AiErr::Blocked(format!("Empty answer from OpenAI{why}")));
    }
    Ok(CallResult { text, file_sent: file_of(o, u).is_some() })
}

pub fn choose_api(provider: &str, endpoint: &str, caps: &ModelCaps) -> &'static str {
    match provider {
        "anthropic" => "messages",
        "openai" => "responses",
        _ => match endpoint {
            "generateContent" => "generateContent",
            "interactions" => "interactions",
            _ => if caps.api == "interactions" { "interactions" } else { "generateContent" },
        },
    }
}

const THINK_NAMES: [&str; 5] = ["thinking", "thinking_level", "thinkinglevel", "effort", "reasoning"];
const OUTPUT_NAMES: [&str; 3] = ["max_output_tokens", "maxoutputtokens", "output token"];
const SCHEMA_FIELDS: [&str; 5] = ["responsejsonschema", "_responsejsonschema", "response_json_schema", "response_format", "responseformat"];

/// The unknown field a 400 names, as (name, path).
pub fn unknown_field(msg: &str) -> Option<(String, String)> {
    let lower = msg.to_lowercase();
    let m = rx!(r"unknown (?:name|field)[^\w$]+([\w.$]+)[^\w$']*(?:at '([^']*)')?").captures(&lower)?;
    let mut parts: Vec<&str> = m[1].split('.').collect();
    let name = parts.pop().unwrap_or("").to_string();
    let mut path: Vec<&str> = vec![m.get(2).map(|x| x.as_str()).unwrap_or("")];
    path.extend(parts);
    Some((name, path.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(".")))
}

/// Send one request with capability fallback. Updates `caps` with what was learned; returns whether it changed.
/// HTTP 400 → drop the refusal fallback, then the schema, then the thinking headroom, then minimal thinking / low
/// effort (Gemini tries level low next), then JSON mode, then the system instruction (whichever the error names
/// first). A 400 about the API key drops nothing.
pub async fn generate(http: &reqwest::Client, cfg: &ProviderCfg<'_>, o: &CallOpts<'_>, caps: &mut ModelCaps) -> Result<(CallResult, bool), AiErr> {
    let api = choose_api(cfg.provider, cfg.endpoint, caps);
    // Claude has no schema-free JSON mode; the prompts ask for JSON and replies are parsed robustly.
    let json_mode = cfg.provider != "anthropic";
    let gemini = cfg.provider == "gemini";
    let json = json_mode && o.json && caps.json != Some(false);
    let pick = if gemini && o.think_minimal { Some(o.level.unwrap_or("minimal")) } else { None };
    let think = o.think_minimal && caps.think_minimal != Some(false) && (!gemini || pick == Some("minimal"));
    let no_level = caps.think_level.as_deref() == Some("none");
    let mut u = Use {
        sys: caps.system != Some(false),
        json,
        think,
        low: gemini && matches!(pick, Some("minimal") | Some("low")) && !think && !no_level,
        deep: match pick { Some("medium") => Some("medium"), Some("high") => Some("high"), _ => None },
        fallback: cfg.provider == "anthropic" && claude_supports_fallback(o.model) && caps.fallback != Some(false),
        schema: json && gemini && o.schema.is_some() && caps.schema != Some(false) && gemini_supports_schema(o.model),
        room: gemini && caps.room != Some(false),
        // Files go to Gemini only (the scheduled task takes them through its own mailbox).
        file: gemini && o.attachment.is_some(),
    };
    let mut changed = false;
    for _ in 0..8 {
        let res = match api {
            "interactions" => call_interactions(http, cfg, o, &u).await,
            "messages" => call_messages(http, cfg, o, &u).await,
            "responses" => call_responses(http, cfg, o, &u).await,
            _ => call_generate_content(http, cfg, o, &u).await,
        };
        let e = match res {
            Ok(mut result) => {
                result.file_sent = u.file && o.attachment.is_some();
                if u.sys && caps.system != Some(true) { caps.system = Some(true); changed = true; }
                if u.json && caps.json != Some(true) { caps.json = Some(true); changed = true; }
                if !json_mode && caps.json != Some(false) { caps.json = Some(false); changed = true; }
                if u.think && caps.think_minimal != Some(true) { caps.think_minimal = Some(true); changed = true; }
                if let Some(l) = if gemini { gemini_level(&u) } else { None } {
                    if (l == "minimal" || l == "low") && caps.think_level.as_deref() != Some(l) { caps.think_level = Some(l.into()); changed = true; }
                }
                if u.fallback && caps.fallback != Some(true) { caps.fallback = Some(true); changed = true; }
                if u.schema && (caps.schema != Some(true) || caps.schema_fails.unwrap_or(0) > 0) { caps.schema = Some(true); caps.schema_fails = Some(0); changed = true; }
                return Ok((result, changed));
            }
            Err(e) => e,
        };
        let body = match &e { AiErr::Http { status: 400, body, .. } if !e.is_auth_400() => body.clone(), _ => return Err(e) };
        // Google's unknown-field errors start "Invalid JSON payload received", which says nothing about JSON mode.
        let msg = body.to_lowercase().replace("invalid json payload", "");
        let names = |k: &[&str]| k.iter().any(|x| msg.contains(x));
        // The file first, whatever the 400 says: a bad or refused file must never cost JSON mode, thinking or the
        // system instruction. The request goes again with text only and nothing else is learned from this error. Only
        // an explicit "this MIME type is not supported" is remembered, for this model.
        if u.file {
            u.file = false;
            if let Some(a) = o.attachment {
                let explicit = names(&["unsupported mime", "mime type", "mimetype"]) && names(&["not supported", "unsupported", "is not allowed"]) && !names(&["response_mime", "responsemimetype"]);
                if explicit {
                    let list = caps.file_rejected.get_or_insert_with(Vec::new);
                    if !list.contains(&a.mime) { list.push(a.mime.clone()); }
                    changed = true;
                }
            }
            continue
        }
        if u.fallback && names(&["fallback"]) { u.fallback = false; caps.fallback = Some(false); changed = true; continue }
        // Schema problems first, so they never cost thinking, JSON mode or the system instruction.
        if u.schema {
            let unknown = unknown_field(&msg);
            if names(&["json mode is not enabled"]) {
                u.schema = false; u.json = false; caps.schema = Some(false); caps.json = Some(false); changed = true; continue
            }
            let field_is_schema = unknown.as_ref().is_some_and(|(n, p)| SCHEMA_FIELDS.contains(&n.as_str()) || (n == "schema" && (p.is_empty() || p == "response_format" || p.ends_with(".response_format"))));
            if field_is_schema || (names(&["schema", "response_format", "responseformat", "structured output"]) && names(&["not supported", "unsupported", "not enabled", "not available"]) && names(&["model"])) {
                u.schema = false; caps.schema = Some(false); changed = true; continue
            }
            // An unknown keyword inside the schema is that schema's problem: this request goes without it.
            if unknown.as_ref().is_some_and(|(_, p)| p.contains("schema") || p.contains("response_format")) { u.schema = false; continue }
            let other: Vec<&str> = THINK_NAMES.iter().chain(OUTPUT_NAMES.iter()).copied().chain(["mime", "json", "system", "instructions"]).collect();
            if names(&["schema", "response_format", "too many states", "constraint", "nesting", "$ref"]) || (unknown.is_none() && !names(&other)) {
                u.schema = false;
                let fails = caps.schema_fails.unwrap_or(0) + 1;
                caps.schema_fails = Some(fails);
                if fails >= 2 { caps.schema = Some(false); }
                changed = true;
                continue
            }
        }
        // Output limit: the thinking headroom is dropped for this model if it was the cause; nothing else is.
        if names(&OUTPUT_NAMES) {
            if gemini_max_output(o, &u) > o.max_output { u.room = false; caps.room = Some(false); changed = true; continue }
            return Err(e);
        }
        let not_other = !names(&["mime", "json", "response_format", "system", "instructions"]);
        // A chosen medium or high level the model rejects: this request goes at the model's own level instead.
        if u.deep.is_some() && (names(&THINK_NAMES) || not_other) { u.deep = None; continue }
        if (u.think || u.low) && (names(&THINK_NAMES) || not_other) {
            if u.think { u.think = false; caps.think_minimal = Some(false); u.low = gemini && caps.think_level.as_deref() != Some("none"); } else { u.low = false; caps.think_level = Some("none".into()); }
            changed = true;
            continue
        }
        if u.json && (names(&["mime", "json", "response_mime_type", "responsemimetype", "json_object", "response_format"]) || !names(&["system", "instructions"])) {
            u.json = false; u.schema = false; caps.json = Some(false); changed = true; continue
        }
        if u.sys { u.sys = false; caps.system = Some(false); changed = true; continue }
        return Err(e);
    }
    Err(AiErr::Other("Request kept failing with HTTP 400 after dropping optional features".into()))
}

/// "Test model": a tiny request that probes system instruction, JSON mode, minimal thinking (or low effort),
/// for Gemini structured output and for Claude the server-side refusal fallback.
pub async fn test_model(http: &reqwest::Client, provider: &str, base_url: &str, key: &str, endpoint: &str, model: &str) -> TestResult {
    let t0 = now_ms();
    let mut notes: Vec<String> = vec![];
    let schema = json!({ "type": "object", "properties": { "ok": { "type": "boolean" }, "reply": { "type": "string" } }, "required": ["ok", "reply"] });
    let opts = CallOpts {
        model, system: "You are a connectivity test. Reply with JSON only.", prompt: "Return exactly {\"ok\":true,\"reply\":\"ready\"}",
        max_output: 256.0, json: true, think_minimal: true, level: None, schema: Some(&schema), cancel: None, attachment: None,
    };
    let gemini = provider == "gemini";
    let fresh = |api: Option<&str>| { let mut c = default_caps(model, provider); if let Some(a) = api { c.api = a.into(); } c };
    let start_api = if gemini { Some(if endpoint == "interactions" { "interactions" } else { "generateContent" }) } else { None };
    let mut caps = fresh(start_api);
    let fail = |api: &str, error: String, mut caps: ModelCaps, notes: Vec<String>| {
        caps.last_error = Some(error.clone());
        caps.tested_at = Some(now_ms());
        TestResult { ok: false, model: model.into(), api: api.into(), latency_ms: now_ms() - t0, reply: None, error: Some(error), caps, notes }
    };
    let ep = caps.api.clone();
    let cfg = |endpoint: &'static str| ProviderCfg { provider, base_url, key, endpoint };
    let first_ep: &'static str = if !gemini { "auto" } else if ep == "interactions" { "interactions" } else { "generateContent" };
    let mut reply;
    match generate(http, &cfg(first_ep), &opts, &mut caps).await {
        Ok((r, _)) => reply = r.text,
        Err(e) => {
            let first = e.message();
            // Failed runs report fresh caps: whatever the fallback dropped on the way was not a real finding.
            if gemini && endpoint == "auto" && model.to_lowercase().starts_with("gemini-3") && !e.is_key_problem() {
                let mut c2 = fresh(Some("interactions"));
                if caps.think_minimal == Some(false) { c2.think_minimal = Some(false); c2.think_level = caps.think_level.clone(); }
                match generate(http, &cfg("interactions"), &opts, &mut c2).await {
                    Ok((r, _)) => {
                        caps = c2;
                        reply = r.text;
                        notes.push(format!("generateContent failed ({first}). The Interactions API worked, so it will be used for this model."));
                    }
                    Err(e2) => return fail("both failed", format!("generateContent: {first} | Interactions: {}", e2.message()), fresh(Some("interactions")), notes),
                }
            } else {
                let start = fresh(start_api);
                let api = start.api.clone();
                return fail(&api, first, start, notes);
            }
        }
    }
    // A schema dropped on the way gets one more try with what was learned; only then does it count as not accepted.
    if gemini && caps.json == Some(true) && caps.schema.is_none() && gemini_supports_schema(model) {
        let mut c2 = caps.clone();
        let ep2: &'static str = if caps.api == "interactions" { "interactions" } else { "generateContent" };
        if let Ok((r, _)) = generate(http, &cfg(ep2), &opts, &mut c2).await { caps = c2; reply = r.text; }
    }
    caps.tested_at = Some(now_ms());
    caps.last_error = None;
    let parsed_ok = extract_json(&reply).ok().is_some_and(|v| v["ok"] == true);
    let (think_label, json_label) = match provider {
        "anthropic" => ("Low effort (output_config.effort)", "JSON mode"),
        "openai" => ("Low reasoning effort", "JSON mode (text.format json_object)"),
        _ => ("Minimal thinking", if caps.api == "interactions" { "JSON mode (response_format)" } else { "JSON mode (responseMimeType)" }),
    };
    notes.push(if caps.system == Some(true) { "System instruction: supported.".into() } else { "System instruction: not accepted; it is folded into the prompt instead.".to_string() });
    if provider == "anthropic" {
        notes.push("JSON: requested in the prompt (Claude has no schema-free JSON mode); replies are parsed robustly.".into());
    } else {
        notes.push(if caps.json == Some(true) { format!("{json_label}: supported.") } else { format!("{json_label}: not accepted; prompt-based JSON with robust parsing is used.") });
    }
    if gemini {
        if caps.schema != Some(true) { caps.schema = Some(false); }
        let label = format!("Structured output ({})", if caps.api == "interactions" { "response_format schema" } else { "responseJsonSchema" });
        let instead = format!("{} with robust parsing is used", if caps.json == Some(true) { "JSON mode" } else { "prompt-based JSON" });
        notes.push(if caps.schema == Some(true) { format!("{label}: supported.") } else if gemini_supports_schema(model) { format!("{label}: not accepted; {instead}.") } else { format!("Structured output: not used for this model type; {instead}.") });
    }
    notes.push(if caps.think_minimal == Some(true) { format!("{think_label}: accepted (saves output tokens).") }
        else if gemini && caps.think_level.as_deref() == Some("low") { format!("{think_label}: not accepted; thinking level low is used instead.") }
        else { format!("{think_label}: not accepted; requests are sent without it.") });
    if provider == "anthropic" {
        notes.push(if caps.fallback == Some(true) { "Refusal fallback: on. If this model declines a request, Anthropic retries it on another Claude model.".into() } else { "Refusal fallback: not available for this model.".to_string() });
    }
    if !parsed_ok { notes.push("The reply was not the exact JSON asked for; parsing and one repair retry will handle this during organizing.".into()); }
    TestResult { ok: true, model: model.into(), api: caps.api.clone(), latency_ms: now_ms() - t0, reply: Some(take_chars(&reply, 300)), error: None, caps, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_forms() {
        assert_eq!(parse_retry_after(Some("12"), "", 0.0), Some(12000.0));
        assert_eq!(parse_retry_after(Some("Wed, 21 Oct 2015 07:28:00 GMT"), "", 1_445_412_475_000.0), Some(5000.0));
        assert_eq!(parse_retry_after(None, r#"{"retryDelay": "17s"}"#, 0.0), Some(17000.0));
        assert_eq!(parse_retry_after(None, "Please retry in 1h2m3.5s.", 0.0), Some(3_723_500.0));
        assert_eq!(parse_retry_after(None, "nothing", 0.0), None);
    }

    #[test]
    fn unknown_field_names() {
        assert_eq!(unknown_field("Unknown name \"anyOf\" at 'response_format.schema'"), Some(("anyof".into(), "response_format.schema".into())));
        assert_eq!(unknown_field("Unknown name \"thinkingConfig\": cannot find field"), Some(("thinkingconfig".into(), String::new())));
    }

    #[test]
    fn backoff_grows_and_honors_server() {
        assert!(backoff_ms(1, None) <= 2000.0 && backoff_ms(1, None) >= 1000.0);
        assert!(backoff_ms(20, None) <= 120_000.0);
        assert_eq!(backoff_ms(1, Some(30_000.0)), 30_000.0);
    }
}
