// Interleaved Study: the native shell. Owns the database, AI queue, keys, timer, menu, tray and notifications;
// the window is a web page that calls `api` and listens for `ev:*` events.
pub mod aijson;
mod commands;
pub mod core;
pub mod deadlines;
pub mod files;
#[cfg(debug_assertions)]
mod devbridge;
pub mod history;
pub mod importer;
pub mod jobs;
mod keys;
pub mod organize;
pub mod preprocess;
pub mod provider;
pub mod queue;
pub mod routine;
pub mod sched;
pub mod session;
pub mod settings;
pub mod store;
pub mod types;
pub mod util;

use crate::core::{Core, Fx};
use crate::queue::{App, AppRef};
use crate::types::SessionState;
use crate::util::{day_key, fmt_clock, now_ms};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// What an alert needs from Settings, readable without the app's lock (alerts are raised while it is held).
#[derive(Clone, Default)]
struct NotifyPrefs { sound: String, bounce: bool, badge: bool }
struct Prefs(Arc<Mutex<NotifyPrefs>>);

pub(crate) fn sync_prefs(h: &AppHandle, app: &AppRef) {
    let n = app.lock().settings.notify.clone();
    if let Some(p) = h.try_state::<Prefs>() { *p.0.lock().unwrap_or_else(|e| e.into_inner()) = NotifyPrefs { sound: n.sound, bounce: n.bounce, badge: n.badge }; }
}

#[tauri::command]
async fn api(app: AppHandle, state: tauri::State<'_, AppRef>, method: String, args: Vec<Value>) -> Result<Value, String> {
    let st: AppRef = state.inner().clone();
    commands::dispatch(&app, &st, &method, args).await
}

fn focus_window(h: &AppHandle) {
    if let Some(w) = h.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn tray_title(s: Option<&SessionState>, now: f64) -> String {
    let Some(s) = s else { return String::new() };
    let left = (s.end_at - s.paused_at.unwrap_or(now)).max(0.0);
    let timed = s.phase == "work" || s.phase == "break";
    match s.phase.as_str() {
        _ if timed && s.paused_at.is_some() => "Paused".into(),
        "work" => fmt_clock(left),
        "break" => format!("Break {}", fmt_clock(left)),
        "rate" => "Rate".into(),
        "next" => "Next".into(),
        "done" => "Save?".into(),
        _ => String::new(),
    }
}

fn update_tray(h: &AppHandle, s: Option<&SessionState>) {
    let Some(tray) = h.tray_by_id("main") else { return };
    let title = tray_title(s, now_ms());
    let _ = tray.set_title(if title.is_empty() { None } else { Some(format!(" {title}")) });
    let tip = match s.map(|s| s.topic_name.as_str()).filter(|n| !n.is_empty()) { Some(n) => format!("Interleaved Study: {n}"), None => "Interleaved Study".into() };
    let _ = tray.set_tooltip(Some(tip));
}

fn data_dir(h: &AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("INTERLEAVED_STUDY_DATA") { if !p.trim().is_empty() { return PathBuf::from(p) } }
    h.path().data_dir().unwrap_or_else(|_| PathBuf::from(".")).join("Interleaved Study 2")
}

/// First launch: bring over the library of the earlier (Electron) app. The old folder is only read, never changed.
fn adopt_old_library(h: &AppHandle, dir: &PathBuf) {
    if dir.join("study.sqlite").exists() || std::env::var("INTERLEAVED_STUDY_DATA").is_ok() { return }
    let Ok(base) = h.path().data_dir() else { return };
    let old = base.join("Interleaved Study");
    if !old.join("study.sqlite").exists() { return }
    for name in ["study.sqlite", "study.sqlite-wal", "study.sqlite-shm"] {
        if old.join(name).exists() { let _ = std::fs::copy(old.join(name), dir.join(name)); }
    }
}

/// Remember where the window is and how big, for the next launch. Called when it closes and when the app quits
/// (Cmd+Q never closes the window first). A full-screen or minimized window keeps the frame saved before.
fn save_frame(h: &AppHandle, app: &AppRef) {
    let Some(w) = h.get_webview_window("main") else { return };
    if w.is_fullscreen().unwrap_or(false) || w.is_minimized().unwrap_or(false) { return }
    let scale = w.scale_factor().unwrap_or(1.0);
    if let (Ok(pos), Ok(size)) = (w.outer_position(), w.inner_size()) {
        if size.width == 0 || size.height == 0 { return }
        app.lock().store.kv_set("window", &json!({ "x": pos.x as f64 / scale, "y": pos.y as f64 / scale, "width": size.width as f64 / scale, "height": size.height as f64 / scale }));
    }
}

fn build_window(h: &AppHandle, app: &AppRef) -> tauri::Result<()> {
    let saved = app.lock().store.kv_get::<Value>("window");
    let num = |k: &str| saved.as_ref().and_then(|v| v[k].as_f64());
    let (w, ht) = (num("width").filter(|v| *v >= 600.0).unwrap_or(1320.0), num("height").filter(|v| *v >= 400.0).unwrap_or(860.0));
    let mut b = WebviewWindowBuilder::new(h, "main", WebviewUrl::default())
        .title("Interleaved Study")
        .inner_size(w, ht)
        .min_inner_size(960.0, 620.0)
        .visible(false);
    #[cfg(target_os = "macos")]
    { b = b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true); }
    if let (Some(x), Some(y)) = (num("x"), num("y")) { b = b.position(x, y); } else { b = b.center(); }
    let win = b.build()?;
    // A frame saved on a display that is gone: bring the window back.
    if win.current_monitor().ok().flatten().is_none() { let _ = win.center(); }
    commands::apply_theme(h, &app.lock().settings.theme.clone());
    let _ = win.show();
    let (h2, a2) = (h.clone(), app.clone());
    win.on_window_event(move |e| match e {
        WindowEvent::CloseRequested { api, .. } => {
            // Closing the window keeps the app (and the menu bar timer) running, as on any Mac.
            save_frame(&h2, &a2);
            if let Some(w) = h2.get_webview_window("main") { let _ = w.hide(); }
            api.prevent_close();
        }
        WindowEvent::Focused(true) => { if let Some(w) = h2.get_webview_window("main") { let _ = w.set_badge_label(None); } }
        _ => {}
    });
    Ok(())
}

fn build_menu(h: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, label: &str, accel: Option<&str>| {
        let mut b = MenuItemBuilder::with_id(id, label);
        if let Some(a) = accel { b = b.accelerator(a); }
        b.build(h)
    };
    let app_menu = SubmenuBuilder::new(h, "Interleaved Study")
        .about(None).separator().item(&item("nav:settings", "Settings…", Some("CmdOrCtrl+,"))?).separator()
        .services().separator().hide().hide_others().show_all().separator().quit().build()?;
    let file = SubmenuBuilder::new(h, "File")
        .item(&item("menu:import-files", "Import Lessons…", Some("CmdOrCtrl+O"))?)
        .item(&item("menu:import-folder", "Import Folder…", Some("CmdOrCtrl+Shift+O"))?)
        .separator()
        .item(&item("backup:export", "Export Backup…", None)?)
        .item(&item("backup:import", "Restore from Backup…", None)?)
        .separator().close_window().build()?;
    let edit = SubmenuBuilder::new(h, "Edit").undo().redo().separator().cut().copy().paste().select_all().build()?;
    let view = SubmenuBuilder::new(h, "View")
        .item(&item("nav:today", "Today", Some("CmdOrCtrl+1"))?)
        .item(&item("nav:session", "Study", Some("CmdOrCtrl+2"))?)
        .item(&item("nav:board", "Library", Some("CmdOrCtrl+3"))?)
        .item(&item("nav:plan", "Plan", Some("CmdOrCtrl+4"))?)
        .item(&item("nav:progress", "Progress", Some("CmdOrCtrl+5"))?)
        .item(&item("nav:queue", "AI Queue", Some("CmdOrCtrl+6"))?)
        .separator().fullscreen().build()?;
    let study = SubmenuBuilder::new(h, "Study")
        .item(&item("nav:session:quickstart", "Start Next Suggested Session", Some("CmdOrCtrl+Shift+S"))?)
        .item(&item("timer:toggle", "Pause or Resume Timer", Some("CmdOrCtrl+Shift+P"))?)
        .build()?;
    let window = SubmenuBuilder::new(h, "Window").minimize().maximize().build()?;
    h.set_menu(MenuBuilder::new(h).items(&[&app_menu, &file, &edit, &view, &study, &window]).build()?)?;
    h.on_menu_event(|h, ev| {
        let id = ev.id().as_ref().to_string();
        let app: AppRef = h.state::<AppRef>().inner().clone();
        if let Some(rest) = id.strip_prefix("nav:") {
            let mut parts = rest.splitn(2, ':');
            focus_window(h);
            let _ = h.emit("ev:nav", json!({ "view": parts.next(), "action": parts.next() }));
        } else if let Some(cmd) = id.strip_prefix("menu:") {
            focus_window(h);
            let _ = h.emit("ev:menu", cmd);
        } else if id == "timer:toggle" {
            let mut c = app.lock();
            if c.session.as_ref().is_some_and(|s| s.paused_at.is_some()) { c.session_resume() } else { c.session_pause() }
        } else if let Some(which) = id.strip_prefix("backup:") {
            // Backup commands run straight from the File menu; the save and open panels are their own feedback.
            let (h, method) = (h.clone(), if which == "export" { "exportBackup" } else { "importBackup" });
            tauri::async_runtime::spawn(async move {
                let err = match commands::dispatch(&h, &app, method, vec![]).await {
                    Err(e) => Some(e),
                    Ok(v) if v["ok"] == false => Some(v["error"].as_str().unwrap_or("").to_string()),
                    Ok(_) => None,
                };
                if let Some(e) = err { commands::notice(&h, "The backup did not finish.".into(), e).await; }
            });
        }
    });
    Ok(())
}

fn build_tray(h: &AppHandle) {
    let Ok(icon) = tauri::image::Image::from_bytes(include_bytes!("../icons/trayTemplate@2x.png")) else { return };
    let _ = TrayIconBuilder::with_id("main").icon(icon).icon_as_template(true).tooltip("Interleaved Study")
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev { focus_window(tray.app_handle()); }
        })
        .build(h);
}

fn setup(h: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let dir = data_dir(h);
    std::fs::create_dir_all(&dir)?;
    adopt_old_library(h, &dir);
    let store = store::Store::open(&dir.join("study.sqlite"))?;

    let prefs = Arc::new(Mutex::new(NotifyPrefs::default()));
    h.manage(Prefs(prefs.clone()));
    let (he, ha) = (h.clone(), h.clone());
    let data_pending = Arc::new(AtomicBool::new(false));
    let fx = Fx {
        emit: Arc::new(move |ev, payload| {
            if ev == "data" {
                // Many changes in a row become one refresh.
                if data_pending.swap(true, Ordering::SeqCst) { return }
                let (h, flag) = (he.clone(), data_pending.clone());
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                    flag.store(false, Ordering::SeqCst);
                    #[cfg(debug_assertions)]
                    devbridge::publish("data", &Value::Null);
                    let _ = h.emit("ev:data", Value::Null);
                });
                return;
            }
            if ev == "session" { update_tray(&he, serde_json::from_value::<Option<SessionState>>(payload.clone()).ok().flatten().as_ref()); }
            #[cfg(debug_assertions)]
            devbridge::publish(ev, &payload);
            let _ = he.emit(&format!("ev:{ev}"), payload);
        }),
        alert: Arc::new(move |title, body| {
            let p = prefs.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let mut n = ha.notification().builder().title(title).body(body);
            if !p.sound.is_empty() && p.sound != "none" { n = n.sound(p.sound.clone()); }
            let _ = n.show();
            if let Some(w) = ha.get_webview_window("main") {
                if !w.is_focused().unwrap_or(false) {
                    if p.bounce { let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational)); }
                    if p.badge { let _ = w.set_badge_label(Some("1".into())); }
                }
            }
        }),
    };
    let mut core = Core::new(store, fx);
    core.secrets = keys::load();
    core.name_courses_by_code();
    core.snapshot_today();
    core.queue_start();
    let app: AppRef = App::new(core, dir);
    h.manage(app.clone());
    sync_prefs(h, &app);

    #[cfg(debug_assertions)]
    devbridge::start(h.clone(), app.clone());
    build_menu(h)?;
    build_window(h, &app)?;
    build_tray(h);
    { let c = app.lock(); update_tray(h, c.session.as_ref()); }

    // The queue runs for the life of the app; batches a scheduled task was already working on are picked up again.
    let a = app.clone();
    tauri::async_runtime::spawn(async move {
        routine::resume(&a);
        queue::run_loop(a).await;
    });
    // One-second heartbeat: the timer, the menu bar countdown and the day rollover.
    let (a, h2) = (app.clone(), h.clone());
    tauri::async_runtime::spawn(async move {
        let mut last_day = day_key(now_ms());
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let mut c = a.lock();
            c.tick();
            update_tray(&h2, c.session.as_ref());
            let today = day_key(now_ms());
            if today != last_day {
                last_day = today;
                if c.snapshot_today() { c.emit_data(); }
            }
        }
    });
    Ok(())
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![api])
        .setup(|app| setup(app.handle()))
        .build(tauri::generate_context!())
        .expect("Interleaved Study could not start");
    app.run(|h, ev| match ev {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => focus_window(h),
        // Cmd+Q ends the app without closing the window first, so its frame is saved here. A window hidden by its
        // close button saved its frame already.
        RunEvent::Exit => {
            if let Some(a) = h.try_state::<AppRef>() {
                if h.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false)) { save_frame(h, a.inner()); }
                a.lock().store.flush();
            }
        }
        _ => {}
    });
}
