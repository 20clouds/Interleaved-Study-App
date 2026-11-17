// Development only (debug builds, and only when INTERLEAVED_STUDY_DEV_PORT is set): the same `api` calls and
// events over loopback HTTP, so the page can be opened in an ordinary browser against the real core.
use crate::queue::AppRef;
use serde_json::{json, Value};
use std::sync::OnceLock;
use tauri::AppHandle;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;

static EVENTS: OnceLock<broadcast::Sender<String>> = OnceLock::new();

pub fn publish(ev: &str, payload: &Value) {
    if let Some(tx) = EVENTS.get() { let _ = tx.send(json!({ "event": ev, "payload": payload }).to_string()); }
}

const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: content-type\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\n";

pub fn start(h: AppHandle, app: AppRef) {
    let Some(port) = std::env::var("INTERLEAVED_STUDY_DEV_PORT").ok().and_then(|p| p.parse::<u16>().ok()) else { return };
    let (tx, _) = broadcast::channel(256);
    let _ = EVENTS.set(tx);
    tauri::async_runtime::spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await else { return };
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { continue };
            let (h, app) = (h.clone(), app.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                let head_end = loop {
                    let Ok(n) = sock.read(&mut tmp).await else { return };
                    if n == 0 { return }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i + 4 }
                    if buf.len() > 65536 { return }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let first = head.lines().next().unwrap_or("");
                if first.starts_with("OPTIONS") {
                    let _ = sock.write_all(format!("HTTP/1.1 204 No Content\r\n{CORS}Content-Length: 0\r\n\r\n").as_bytes()).await;
                } else if first.starts_with("GET /events") {
                    let Some(mut rx) = EVENTS.get().map(|t| t.subscribe()) else { return };
                    if sock.write_all(format!("HTTP/1.1 200 OK\r\n{CORS}Content-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n").as_bytes()).await.is_err() { return }
                    while let Ok(msg) = rx.recv().await {
                        if sock.write_all(format!("data: {msg}\n\n").as_bytes()).await.is_err() { return }
                    }
                } else if first.starts_with("POST /api") {
                    let len = head.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                    while buf.len() < head_end + len {
                        let Ok(n) = sock.read(&mut tmp).await else { return };
                        if n == 0 { break }
                        buf.extend_from_slice(&tmp[..n]);
                    }
                    let req: Value = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
                    let args = req["args"].as_array().cloned().unwrap_or_default();
                    let out = match crate::commands::dispatch(&h, &app, req["method"].as_str().unwrap_or(""), args).await {
                        Ok(v) => json!({ "ok": true, "value": v }),
                        Err(e) => json!({ "ok": false, "error": e }),
                    }.to_string();
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\n{CORS}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{out}", out.len()).as_bytes()).await;
                }
            });
        }
    });
}
