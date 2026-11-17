// The C interface the Swift app calls (app/Engine/engine.h declares it). Three entry points: start the engine,
// call one `api` method with JSON arguments, and stop. Events come back through the callback given to start.
use crate::core::{Core, Fx};
use crate::queue::{App, AppRef};
use crate::util::{day_key, now_ms};
use crate::{commands, host, keys, queue, routine, store};
use serde_json::{json, Value};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// An event for the app: its name and a JSON payload, both valid only during the call.
pub type EventFn = extern "C" fn(*const c_char, *const c_char);
/// The answer to one call: the context given to `is_call`, whether it succeeded, and JSON (the value, or the error
/// message as a JSON string), valid only during the call.
pub type ReplyFn = extern "C" fn(*mut c_void, bool, *const c_char);

struct Engine {
    rt: tokio::runtime::Runtime,
    app: AppRef,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();

fn cstr(p: *const c_char) -> String {
    if p.is_null() { return String::new() }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn send(f: EventFn, ev: &str, payload: &Value) {
    let (Ok(e), Ok(p)) = (CString::new(ev), CString::new(payload.to_string())) else { return };
    f(e.as_ptr(), p.as_ptr());
}

/// Open the library in `data_dir` and start the queue and the one-second timer. Returns false when the database
/// cannot be opened; calling it again after a success does nothing.
#[no_mangle]
pub extern "C" fn is_start(data_dir: *const c_char, on_event: EventFn, on_host: host::HostFn) -> bool {
    if ENGINE.get().is_some() { return true }
    host::set(on_host);
    let dir = PathBuf::from(cstr(data_dir));
    if std::fs::create_dir_all(&dir).is_err() { return false }
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread().enable_all().build() else { return false };
    let Ok(st) = store::Store::open(&dir.join("study.sqlite")) else { return false };

    let data_pending = Arc::new(AtomicBool::new(false));
    let handle = rt.handle().clone();
    let fx = Fx {
        emit: Arc::new(move |ev, payload| {
            if ev == "data" {
                // Many changes in a row become one refresh.
                if data_pending.swap(true, Ordering::SeqCst) { return }
                let flag = data_pending.clone();
                handle.spawn(async move {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    flag.store(false, Ordering::SeqCst);
                    #[cfg(debug_assertions)]
                    crate::devbridge::publish("data", &Value::Null);
                    send(on_event, "data", &Value::Null);
                });
                return;
            }
            #[cfg(debug_assertions)]
            crate::devbridge::publish(ev, &payload);
            send(on_event, ev, &payload);
        }),
        // The app posts the notification and bounces or badges the Dock as Settings say.
        alert: Arc::new(move |title, body| send(on_event, "alert", &json!({ "title": title, "body": body }))),
    };
    let mut core = Core::new(st, fx);
    core.secrets = keys::load();
    core.name_courses_by_code();
    core.snapshot_today();
    core.queue_start();
    let app: AppRef = App::new(core, dir);

    rt.spawn({
        let a = app.clone();
        async move {
            // The queue runs for the life of the app; batches a scheduled task was already working on are picked up again.
            routine::resume(&a);
            queue::run_loop(a).await;
        }
    });
    rt.spawn({
        let a = app.clone();
        async move {
            // One-second heartbeat: the timer and the day rollover.
            let mut last_day = day_key(now_ms());
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                let mut c = a.lock();
                c.tick();
                let today = day_key(now_ms());
                if today != last_day {
                    last_day = today;
                    if c.snapshot_today() { c.emit_data(); }
                }
            }
        }
    });
    #[cfg(debug_assertions)]
    {
        let _guard = rt.enter();
        crate::devbridge::start(app.clone());
    }
    let _ = ENGINE.set(Engine { rt, app });
    true
}

/// Run one `api` method. `args_json` is a JSON array; the reply arrives on an engine thread.
#[no_mangle]
pub extern "C" fn is_call(method: *const c_char, args_json: *const c_char, ctx: *mut c_void, reply: ReplyFn) {
    let (method, args) = (cstr(method), cstr(args_json));
    let ctx = ctx as usize; // handed back untouched
    let answer = move |ok: bool, v: Value| {
        let Ok(c) = CString::new(v.to_string()) else { return };
        reply(ctx as *mut c_void, ok, c.as_ptr());
    };
    let Some(e) = ENGINE.get() else { return answer(false, json!("The engine is not running.")) };
    let args: Vec<Value> = serde_json::from_str(&args).unwrap_or_default();
    let app = e.app.clone();
    e.rt.spawn(async move {
        match commands::dispatch(&app, &method, args).await {
            Ok(v) => answer(true, v),
            Err(err) => answer(false, json!(err)),
        }
    });
}

/// Write everything to disk before the app quits.
#[no_mangle]
pub extern "C" fn is_stop() {
    if let Some(e) = ENGINE.get() { e.app.lock().store.flush(); }
}
