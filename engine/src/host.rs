// What the engine asks of the Mac app around it: a confirmation sheet, or an open or save panel. The app answers
// on its main thread, so every question waits on a blocking worker, never on the async ones.
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Takes a JSON request; returns a JSON answer the app allocated with malloc (freed here), or null.
pub type HostFn = extern "C" fn(*const c_char) -> *mut c_char;

static HOST: OnceLock<HostFn> = OnceLock::new();

extern "C" {
    fn free(p: *mut c_void);
}

pub fn set(f: HostFn) { let _ = HOST.set(f); }

async fn ask(req: Value) -> Value {
    let Some(f) = HOST.get().copied() else { return Value::Null };
    tokio::task::spawn_blocking(move || {
        let Ok(c) = CString::new(req.to_string()) else { return Value::Null };
        let out = f(c.as_ptr());
        if out.is_null() { return Value::Null }
        let v = unsafe { CStr::from_ptr(out) }.to_str().ok().and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null);
        unsafe { free(out as *mut c_void) };
        v
    }).await.unwrap_or(Value::Null)
}

/// A sheet on the window; a destructive action is never the Return-key default.
pub async fn confirm(message: String, detail: String, ok: String, danger: bool) -> bool {
    ask(json!({ "op": "confirm", "message": message, "detail": detail, "ok": ok, "danger": danger })).await == json!(true)
}

pub enum Pick { Files, Folders, Sheet, Backup, SaveBackup(String) }

/// The chosen paths; empty when the person cancelled.
pub async fn pick(what: Pick) -> Vec<PathBuf> {
    let req = match what {
        Pick::Files => json!({ "op": "open", "title": "Choose lesson files", "types": crate::importer::PICK_EXT, "multiple": true }),
        Pick::Folders => json!({ "op": "open", "title": "Choose a folder of lessons", "folders": true, "multiple": true }),
        Pick::Sheet => json!({ "op": "open", "title": "Import deadlines", "types": ["xlsx", "xls", "csv"] }),
        Pick::Backup => json!({ "op": "open", "title": "Import backup", "types": ["json"] }),
        Pick::SaveBackup(name) => json!({ "op": "save", "title": "Export backup", "name": name, "types": ["json"] }),
    };
    ask(req).await.as_array().map(|l| l.iter().filter_map(|p| p.as_str().map(PathBuf::from)).collect()).unwrap_or_default()
}
