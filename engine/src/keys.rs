// API keys and tokens live in the macOS Keychain as one item (a small JSON map), so the system asks at most once.
// They are read once at launch and kept in memory; nothing secret is ever written to the database or a backup.
use std::collections::HashMap;

const SERVICE: &str = "Interleaved Study";
const ACCOUNT: &str = "api-keys";

/// Debug builds only: `INTERLEAVED_STUDY_KEYS='{"gemini":"…"}'` supplies the secrets and the Keychain is left alone.
/// A release build never reads it, so a stray or planted environment variable cannot replace the stored keys.
#[cfg(debug_assertions)]
fn dev_override() -> Option<HashMap<String, String>> {
    serde_json::from_str(&std::env::var("INTERLEAVED_STUDY_KEYS").ok()?).ok()
}

#[cfg(not(debug_assertions))]
fn dev_override() -> Option<HashMap<String, String>> { None }

pub fn load() -> HashMap<String, String> {
    if let Some(m) = dev_override() { return m }
    match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

pub fn save(all: &HashMap<String, String>) -> Result<(), String> {
    if dev_override().is_some() { return Ok(()) }
    let kept: HashMap<&String, &String> = all.iter().filter(|(_, v)| !v.is_empty()).collect();
    if kept.is_empty() {
        let _ = security_framework::passwords::delete_generic_password(SERVICE, ACCOUNT);
        return Ok(());
    }
    let json = serde_json::to_vec(&kept).map_err(|e| e.to_string())?;
    security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, &json).map_err(|e| format!("The Keychain did not accept the key ({e})."))
}
