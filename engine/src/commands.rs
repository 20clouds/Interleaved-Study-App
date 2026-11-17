// Every call the app can make: one method name and its arguments in JSON, one JSON answer. The Swift app calls these
// through ffi.rs (Engine.swift lists them).
use crate::core::{AddTopic, NewJob};
use crate::deadlines::{chunk_markdown, deadline_key, read_sheets, sheets_to_markdown, CHUNK_ROWS, MAX_SHEET_MARKDOWN};
use crate::jobs::QUIZ_TEXT_CHARS;
use crate::queue::AppRef;
use crate::sched::ReviewInput;
use crate::settings::{self, base_url_problem, deep_merge, default_base_url, key_name, merge_settings, provider_label, url_host, PROVIDERS, SECRETS};
use crate::types::*;
use crate::util::{now_ms, sha256, take_chars, today};
use crate::{importer, keys, provider, routine, rx};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use crate::host::{confirm, pick, Pick};
use std::path::PathBuf;

fn s(a: &[Value], i: usize) -> String { a.get(i).and_then(|x| x.as_str()).unwrap_or("").to_string() }
fn opt_s(a: &[Value], i: usize) -> Option<String> { a.get(i).and_then(|x| x.as_str()).map(|x| x.to_string()) }
fn f(a: &[Value], i: usize) -> Option<f64> { a.get(i).and_then(|x| x.as_f64()) }
fn b(a: &[Value], i: usize) -> bool { a.get(i).and_then(|x| x.as_bool()).unwrap_or(false) }
fn obj(a: &[Value], i: usize) -> Value { a.get(i).cloned().unwrap_or(Value::Null) }
fn parse<T: DeserializeOwned>(a: &[Value], i: usize, what: &str) -> Result<T, String> {
    serde_json::from_value(obj(a, i)).map_err(|_| format!("Unexpected {what}."))
}
fn strs(a: &[Value], i: usize) -> Vec<String> {
    a.get(i).and_then(|x| x.as_array()).map(|l| l.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default()
}
fn val<T: serde::Serialize>(v: T) -> Result<Value, String> { serde_json::to_value(v).map_err(|e| e.to_string()) }
const DONE: Result<Value, String> = Ok(Value::Null);

fn caps_info(app: &AppRef) -> Value {
    json!({
        "storeKind": "sqlite", "storeError": null, "safeStorage": true, "notifications": true, "tray": true, "dock": true,
        "dataPath": app.data_dir.to_string_lossy(), "markitdown": null
    })
}

fn key_flags(app: &AppRef) -> Value {
    let c = app.lock();
    Value::Object(SECRETS.iter().map(|p| (p.to_string(), json!(c.secret(p).is_some()))).collect())
}

async fn wait_job(app: &AppRef, job: Job) -> Result<Value, String> {
    let rx = app.lock().enqueue_wait(&job);
    app.kick();
    rx.await.map_err(|_| "Cancelled".to_string())?
}

pub async fn dispatch(app: &AppRef, method: &str, a: Vec<Value>) -> Result<Value, String> {
    let a = a.as_slice();
    match method {
        // ---------- setup ----------
        "bootstrap" => {
            let (settings, has_key, model_caps) = { let c = app.lock(); (c.settings_raw.clone(), c.active_key().is_some(), c.store.kv_get::<Value>("modelCaps").unwrap_or(json!({}))) };
            Ok(json!({ "settings": settings, "hasKey": has_key, "keys": key_flags(app), "caps": caps_info(app), "modelCaps": model_caps }))
        }
        "capsCheck" => Ok(caps_info(app)),
        // How each file type reaches the current provider and model: as the original file, as text, or not at all.
        "fileSupport" => {
            let c = app.lock();
            let ai = &c.settings.ai;
            val(json!({ "provider": crate::files::provider_name(ai), "model": if ai.is_routine() { "" } else { ai.model.as_str() }, "types": crate::files::support_table(ai, &c.caps(&ai.model)) }))
        }
        "setSettings" => {
            let patch = obj(a, 0);
            // Requests carry the API key, so a new address is checked here and a non-official host needs the
            // person's confirmation in a native dialog.
            if let Some(urls) = patch.pointer("/ai/baseUrls").and_then(|x| x.as_object()) {
                for (p, raw) in urls {
                    let Some(raw) = raw.as_str().filter(|_| PROVIDERS.contains(&p.as_str()) && p != "routine") else { return Err("Unknown provider.".into()) };
                    if let Some(problem) = base_url_problem(raw) { return Err(format!("That base URL {problem}.")) }
                    let host = url_host(raw);
                    let current = app.lock().settings.ai.base_url(p);
                    if host.is_empty() || host == url_host(default_base_url(p)) || host == url_host(&current) { continue }
                    let ok = confirm(format!("Send your {} requests and API key to {host}?", provider_label(p)),
                        "Lesson text and your API key will go to this address instead of the official one. Only continue if you run or trust it.".into(), "Use this address".into(), true).await;
                    if !ok { return Err("Base URL not changed.".into()) }
                }
            }
            if let Some(u) = patch.pointer("/ai/routine/fireUrl").and_then(|x| x.as_str()).filter(|u| !u.trim().is_empty()) {
                if let Some(p) = routine::fire_url_problem(u) { return Err(format!("That task URL {p}.")) }
            }
            if let Some(r) = patch.pointer("/ai/routine/repo").and_then(|x| x.as_str()).filter(|r| !r.trim().is_empty()) {
                if !routine::repo_ok(r) { return Err("The mailbox repository should look like owner/name.".into()) }
            }
            let out = {
                let mut c = app.lock();
                c.settings_raw = merge_settings(Some(&deep_merge(&c.settings_raw, &patch)));
                c.save_settings();
                c.settings_raw.clone()
            };
            app.kick();
            Ok(out)
        }
        "setKey" => {
            let key = s(a, 0).trim().to_string();
            let p = opt_s(a, 1).unwrap_or_else(|| app.lock().settings.ai.provider.clone());
            if !SECRETS.contains(&p.as_str()) { return Err("Unknown provider.".into()) }
            if key.is_empty() { return Ok(json!({ "ok": false, "error": "The key is empty." })) }
            // A task token missing a character is rejected by Claude only when the first run starts; catch it here.
            if p == "routine" && !key.starts_with("sk-ant-oat01-") {
                return Ok(json!({ "ok": false, "error": "That does not look like a task token: it should start with sk-ant-oat01-. Copy the whole token again (the first letter is easy to miss)." }));
            }
            let all = { let mut c = app.lock(); c.secrets.insert(p.clone(), key); c.secrets.clone() };
            if let Err(e) = tokio::task::spawn_blocking(move || keys::save(&all)).await.unwrap_or_else(|e| Err(e.to_string())) {
                app.lock().secrets.remove(&p);
                return Ok(json!({ "ok": false, "error": e }));
            }
            {
                let mut c = app.lock();
                let waiting_on_key = c.q.pause_reason.as_deref().is_some_and(|r| r.contains("API key") || r.contains("token") || r.contains("GitHub"));
                if waiting_on_key && c.q.rate_limit.is_none() && c.active_key().is_some() { c.queue_resume(); }
            }
            app.kick();
            Ok(json!({ "ok": true }))
        }
        "clearKey" => {
            let p = opt_s(a, 0).unwrap_or_else(|| app.lock().settings.ai.provider.clone());
            if !SECRETS.contains(&p.as_str()) { return Err("Unknown provider.".into()) }
            let all = { let mut c = app.lock(); c.secrets.remove(&p); c.secrets.clone() };
            let _ = tokio::task::spawn_blocking(move || keys::save(&all)).await;
            app.kick();
            DONE
        }
        "testModel" => {
            let (ai, key) = { let c = app.lock(); (c.settings.ai.clone(), c.secret(&c.settings.ai.provider).map(|k| k.to_string())) };
            if ai.is_routine() {
                let r = routine::check(app).await;
                app.lock().save_caps(&r.caps);
                return val(r);
            }
            let model = opt_s(a, 0).map(|m| m.trim().to_string()).filter(|m| !m.is_empty()).unwrap_or_else(|| ai.model.clone());
            let Some(key) = key else {
                let caps = app.lock().caps(&model);
                return val(TestResult { ok: false, model, api: "-".into(), latency_ms: 0.0, reply: None, error: Some(format!("Add your {} first.", key_name(&ai.provider))), caps, notes: vec![] });
            };
            let res = provider::test_model(&app.http, &ai.provider, &ai.base_url(&ai.provider), &key, &ai.endpoint, &model).await;
            {
                // A failed test only records the error; what was learned about the model before stays.
                let c = app.lock();
                if res.ok { c.save_caps(&res.caps); } else {
                    let mut old = c.caps(&model);
                    old.last_error = res.caps.last_error.clone().or_else(|| res.error.clone());
                    old.tested_at = res.caps.tested_at.or(Some(now_ms()));
                    c.save_caps(&old);
                }
            }
            val(res)
        }
        "routineInfo" => Ok(json!({ "prompt": routine::ROUTINE_PROMPT, "guide": routine::MAILBOX_GUIDE, "github": routine::gh_source(app).await })),
        "routineCreateRepo" => {
            let name = { let n = s(a, 0); if n.trim().is_empty() { "interleaved-study-mailbox".to_string() } else { n } };
            match routine::create_repo(app, &name).await {
                Ok(repo) => {
                    let out = {
                        let mut c = app.lock();
                        c.settings_raw = merge_settings(Some(&deep_merge(&c.settings_raw, &json!({ "ai": { "routine": { "repo": repo } } }))));
                        c.save_settings();
                        c.settings_raw.clone()
                    };
                    app.kick();
                    Ok(json!({ "ok": true, "repo": repo, "settings": out }))
                }
                Err(e) => Ok(json!({ "ok": false, "error": e })),
            }
        }

        // ---------- library ----------
        "snapshot" => Ok(app.lock().data_snapshot()),
        "topicText" => {
            let c = app.lock();
            let text = c.topic_text(&s(a, 0));
            let paras: Vec<&str> = if text.is_empty() { vec![] } else { rx!(r"\n{2,}").split(&text).collect() };
            Ok(json!({ "text": text, "paras": paras, "excerpt": c.topic_excerpt(&s(a, 0)) }))
        }
        "search" => val(app.lock().store.search(&s(a, 0), 50)),
        "renameTopic" => { let c = app.lock(); c.rename_topic(&s(a, 0), &s(a, 1)); c.emit_data(); DONE }
        "updateTopic" => { let c = app.lock(); c.update_topic(&s(a, 0), &obj(a, 1)); c.emit_data(); DONE }
        "mergeTopics" => { let c = app.lock(); let id = c.merge_topics(&strs(a, 0), None)?; c.emit_data(); Ok(json!(id)) }
        "splitTopic" => {
            let c = app.lock();
            let id = c.split_topic(&s(a, 0), f(a, 1).unwrap_or(0.0).max(0.0) as usize, opt_s(a, 2).as_deref())?;
            c.emit_data();
            Ok(json!(id))
        }
        "addTopic" => {
            let i = obj(a, 0);
            let g = |k: &str| i.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
            let input = AddTopic { doc_id: g("docId"), course_id: g("courseId"), course_name: g("courseName"), lesson_title: g("lessonTitle"), unit: g("unit"), name: g("name").unwrap_or_default(), text: g("text").unwrap_or_default() };
            if input.text.trim().is_empty() { return Err("Add some text for the topic.".into()) }
            let id = {
                let c = app.lock();
                let t = c.add_topic(&input)?;
                if i["enrich"].as_bool() == Some(true) {
                    c.create_job(NewJob { kind: "enrich", doc_id: Some(t.doc_id.clone()), topic_id: Some(t.id.clone()), label: format!("Describe \"{}\"", t.name), payload: json!({}), est_tokens: (input.text.len() as f64 / 4.0).ceil() + 200.0 });
                }
                c.emit_data();
                t.id
            };
            app.kick();
            Ok(json!(id))
        }
        "deleteTopics" => { let c = app.lock(); c.store.tx(|| for id in strs(a, 0) { c.delete_topic(&id) }); c.emit_data(); DONE }
        "override" => { let c = app.lock(); c.override_topic(&s(a, 0), &obj(a, 1)); c.emit_data(); DONE }
        "reviewTopic" => {
            let rating: Option<Rating> = serde_json::from_value(obj(a, 1)).unwrap_or(None);
            let c = app.lock();
            c.review(&s(a, 0), &ReviewInput { rating, quiz: f(a, 2), minutes: f(a, 3).unwrap_or(0.0), note: opt_s(a, 4), at: now_ms() }, true);
            c.emit_data();
            DONE
        }
        "createCourse" => { let c = app.lock(); let id = c.create_course(&s(a, 0))?.id; c.emit_data(); Ok(json!(id)) }
        "renameCourse" => { let c = app.lock(); c.rename_course(&s(a, 0), &s(a, 1)); c.emit_data(); DONE }
        "deleteCourse" => { let c = app.lock(); c.delete_course(&s(a, 0)); c.emit_data(); c.emit_status(); DONE }
        "renameDoc" => { let c = app.lock(); c.rename_doc(&s(a, 0), &s(a, 1)); c.emit_data(); DONE }
        "deleteDoc" => {
            let id = s(a, 0);
            let mut c = app.lock();
            c.queue_cancel(|j| j.doc_id.as_deref() == Some(id.as_str()));
            c.delete_doc(&id);
            c.emit_data();
            c.emit_status();
            DONE
        }
        "historyCount" => Ok(json!(app.lock().history_count(&s(a, 0)))),
        "reorganize" => { { let c = app.lock(); c.enqueue_organize(&s(a, 0), true); c.emit_data(); } app.kick(); DONE }
        "retryDoc" => {
            let id = s(a, 0);
            {
                let c = app.lock();
                let failed: Vec<Job> = c.store.by_parent::<Job>("jobs", &id).into_iter().filter(|j| j.status == "failed").collect();
                if failed.is_empty() { c.enqueue_organize(&id, false); } else { for j in failed { c.queue_retry(&j.id); } }
                if let Some(mut d) = c.store.get::<Doc>("docs", &id) { d.status = "queued".into(); d.error = None; c.store.put("docs", &d); }
                c.emit_data();
            }
            app.kick();
            DONE
        }

        // ---------- import ----------
        "pickPaths" => {
            let paths = pick(if s(a, 0) == "folder" { Pick::Folders } else { Pick::Files }).await;
            val(paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>())
        }
        "importPaths" => {
            let paths = strs(a, 0);
            let p2 = paths.clone();
            let files = tokio::task::spawn_blocking(move || importer::read_all(&p2)).await.map_err(|e| e.to_string())?;
            let c = app.lock();
            let res = importer::apply(&c, &paths, files, b(a, 1), opt_s(a, 2).as_deref())?;
            c.emit_data();
            val(res)
        }

        // ---------- queue ----------
        "estimate" => { let ids = a.first().filter(|x| x.is_array()).map(|_| strs(a, 0)); val(app.lock().estimate(ids.as_deref())) }
        "startQueue" => {
            let ids = a.first().filter(|x| x.is_array()).map(|_| strs(a, 0));
            {
                let mut c = app.lock();
                for d in c.store.all::<Doc>("docs") {
                    let wanted = match &ids { Some(l) => l.contains(&d.id), None => d.status == "awaiting" || d.status == "needs" };
                    if wanted { c.enqueue_organize(&d.id, false); }
                }
                if c.q.user_paused { c.queue_resume(); }
                c.emit_data();
            }
            app.kick();
            DONE
        }
        "pauseQueue" => { app.lock().queue_pause(); app.kick(); DONE }
        "resumeQueue" => { app.lock().queue_resume(); app.kick(); DONE }
        "queueStatus" => val(app.lock().queue_status()),
        "retryJob" => {
            {
                let c = app.lock();
                let j = c.store.get::<Job>("jobs", &s(a, 0));
                c.queue_retry(&s(a, 0));
                if let Some(mut d) = j.and_then(|j| j.doc_id).and_then(|id| c.store.get::<Doc>("docs", &id)) {
                    if d.status == "needs" { d.status = "queued".into(); d.error = None; c.store.put("docs", &d); }
                }
                c.emit_data();
            }
            app.kick();
            DONE
        }
        "discardJob" => { { let mut c = app.lock(); c.queue_discard(&s(a, 0)); c.emit_data(); } app.kick(); DONE }
        "cancelAi" => {
            let t = obj(a, 0);
            let g = |k: &str| t.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
            let (job_id, kind, topic_id, doc_id) = (g("jobId"), g("kind"), g("topicId"), g("docId"));
            let n = {
                let mut c = app.lock();
                let hit = |want: &Option<String>, have: Option<&str>| want.as_deref().is_none_or(|w| have == Some(w));
                let mut cancelled = c.queue_cancel(|j| hit(&job_id, Some(&j.id)) && hit(&kind, Some(&j.kind)) && hit(&topic_id, j.topic_id.as_deref()) && hit(&doc_id, j.doc_id.as_deref()));
                // Half an organize run is no use: a lesson that lost one window loses the rest and goes back to where it was.
                let docs: std::collections::HashSet<String> = cancelled.iter().filter(|j| j.kind == "organize" || j.kind == "read").filter_map(|j| j.doc_id.clone()).collect();
                if !docs.is_empty() {
                    let more = c.queue_cancel(|j| (j.kind == "organize" || j.kind == "read") && j.doc_id.as_ref().is_some_and(|d| docs.contains(d)));
                    cancelled.extend(more);
                }
                for id in &docs { c.cancel_organize(id); }
                c.emit_data();
                cancelled.len()
            };
            app.kick();
            Ok(json!(n))
        }

        // ---------- quizzes ----------
        "quiz" => {
            let (topic_id, regenerate) = (s(a, 0), b(a, 1));
            let job = {
                let c = app.lock();
                let t = c.store.get::<Topic>("topics", &topic_id).ok_or("Topic not found")?;
                let text = take_chars(&c.topic_text(&topic_id), QUIZ_TEXT_CHARS);
                if let (Some(q), false) = (c.store.get::<Quiz>("quizzes", &topic_id), regenerate) { if q.text_hash == sha256(&text) { return val(q) } }
                c.require_ai_ready()?;
                c.create_job(NewJob { kind: "quiz", doc_id: Some(t.doc_id), topic_id: Some(topic_id), label: format!("Quiz \"{}\"", t.name), payload: json!({ "regenerate": regenerate }), est_tokens: (text.len() as f64 / 4.0).ceil() + 250.0 })
            };
            wait_job(app, job).await
        }
        "grade" => {
            let items = obj(a, 1);
            let job = {
                let c = app.lock();
                c.require_ai_ready()?;
                c.create_job(NewJob { kind: "grade", doc_id: None, topic_id: opt_s(a, 0), label: "Grade short answers".into(), est_tokens: (items.to_string().len() as f64 / 4.0).ceil() + 150.0, payload: json!({ "items": items }) })
            };
            wait_job(app, job).await
        }

        // ---------- session ----------
        "sessionState" => val(app.lock().session.clone()),
        "sessionPreview" => { let pool: Pool = parse(a, 0, "pool")?; val(app.lock().suggest(&pool, b(a, 1), 8)) }
        "sessionStart" => {
            let o = obj(a, 0);
            let pool: Pool = serde_json::from_value(o["pool"].clone()).map_err(|_| "Unexpected pool.".to_string())?;
            let st = app.lock().session_start(o["mode"].as_str().unwrap_or("timed"), pool, o["poolLabel"].as_str().unwrap_or("").to_string(), o["topicId"].as_str().map(|x| x.to_string()))?;
            val(st)
        }
        "sessionPause" => { app.lock().session_pause(); DONE }
        "sessionResume" => { app.lock().session_resume(); DONE }
        "sessionExtend" => { app.lock().session_extend(f(a, 0).unwrap_or(0.0)); DONE }
        "sessionSkip" => { let mut c = app.lock(); c.session_skip(); c.emit_data(); DONE }
        "sessionEndEarly" => { app.lock().session_end_early(); DONE }
        "sessionRate" => {
            let r = obj(a, 0);
            let rating: Option<Rating> = serde_json::from_value(r["rating"].clone()).unwrap_or(None);
            let mut c = app.lock();
            c.session_rate(rating, r["quiz"].as_f64(), r["note"].as_str().map(|x| x.to_string()));
            c.emit_data();
            DONE
        }
        "sessionAccept" => { app.lock().session_accept(opt_s(a, 0)); DONE }
        "sessionSkipBreak" => { app.lock().session_skip_break(); DONE }
        "sessionSkipSuggestion" => { app.lock().session_skip_suggestion(); DONE }
        "sessionEnd" => { let mut c = app.lock(); c.session_end(); c.emit_data(); DONE }
        "sessionSandbox" => { let mut c = app.lock(); c.session_sandbox(b(a, 0)); c.emit_data(); DONE }

        // ---------- deadlines ----------
        "addDeadline" => {
            let d: DeadlineInput = parse(a, 0, "deadline")?;
            if d.title.trim().is_empty() || d.date.is_empty() { return Err("A deadline needs a title and a date.".into()) }
            let c = app.lock();
            c.add_deadline(&d);
            c.emit_data();
            DONE
        }
        "updateDeadline" => {
            let c = app.lock();
            let Some(old) = c.store.get::<Value>("deadlines", &s(a, 0)) else { return DONE };
            let mut merged = deep_merge(&old, &obj(a, 1));
            merged["id"] = json!(s(a, 0));
            let mut n: Deadline = serde_json::from_value(merged).map_err(|_| "Unexpected deadline.".to_string())?;
            n.key = deadline_key(&n.title, &n.date, c.course_name(n.course_id.as_deref()).as_deref());
            c.store.put("deadlines", &n);
            c.emit_data();
            DONE
        }
        "deleteDeadline" => { let c = app.lock(); c.store.del("deadlines", &s(a, 0)); c.emit_data(); DONE }
        "openSheet" => {
            let Some(file) = pick(Pick::Sheet).await.into_iter().next() else { return DONE };
            let f2 = file.clone();
            let sheets = tokio::task::spawn_blocking(move || read_sheets(&f2)).await.map_err(|e| e.to_string())??;
            let path = file.to_string_lossy().to_string();
            app.lock().last_sheet = Some(path.clone());
            Ok(json!({ "path": path, "sheets": sheets }))
        }
        "extractDeadlines" => {
            let file = s(a, 0);
            // Only the spreadsheet the person just picked may be read.
            if app.lock().last_sheet.as_deref() != Some(file.as_str()) { return Err("Pick the spreadsheet again with Import.".into()) }
            app.lock().require_ai_ready()?;
            let f2 = PathBuf::from(&file);
            let markdown = tokio::task::spawn_blocking(move || read_sheets(&f2).map(|s| sheets_to_markdown(&s))).await.map_err(|e| e.to_string())??;
            if markdown.trim().is_empty() { return Err("The spreadsheet is empty.".into()) }
            if markdown.len() > MAX_SHEET_MARKDOWN {
                return Err(format!("This spreadsheet is too large to send to the AI ({}k characters; the limit is {}k). Map its columns manually instead.", markdown.len() / 1000, MAX_SHEET_MARKDOWN / 1000));
            }
            let (job, n) = {
                let c = app.lock();
                let chunks = chunk_markdown(&markdown, CHUNK_ROWS, (c.settings.ai.window_tokens * 4.0) as usize);
                let courses: Vec<String> = c.store.all::<Course>("courses").into_iter().map(|x| x.name).collect();
                let name = PathBuf::from(&file).file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
                let n = chunks.len();
                (c.create_job(NewJob { kind: "deadlines", doc_id: None, topic_id: None, label: format!("Find deadlines in \"{name}\""), est_tokens: (markdown.len() as f64 / 4.0).ceil() + n as f64 * 450.0, payload: json!({ "chunks": chunks, "today": today(), "courses": courses }) }), n)
            };
            let res = wait_job(app, job).await?;
            Ok(json!({ "path": file, "converter": "builtin", "converterNote": null, "items": res["items"], "dropped": res["dropped"], "chunks": n }))
        }
        "importDeadlines" => {
            let items: Vec<DeadlineInput> = parse(a, 0, "deadlines")?;
            let c = app.lock();
            let (mut added, mut skipped) = (0, 0);
            c.store.tx(|| for it in &items { if c.add_deadline(it) { added += 1 } else { skipped += 1 } });
            c.emit_data();
            Ok(json!({ "added": added, "skipped": skipped }))
        }
        "setDayGoal" => { let c = app.lock(); c.set_day_goal(serde_json::from_value(obj(a, 0)).unwrap_or(None)); c.emit_data(); DONE }

        // ---------- backup and system ----------
        "exportBackup" => {
            let Some(path) = pick(Pick::SaveBackup(format!("interleaved-study-backup-{}.json", today()))).await.into_iter().next() else { return DONE };
            let dump = { let c = app.lock(); c.store.flush(); c.store.dump() };
            std::fs::write(&path, dump.to_string()).map_err(|e| format!("Could not write the backup: {e}"))?;
            Ok(json!(path.to_string_lossy()))
        }
        "importBackup" => {
            let Some(path) = pick(Pick::Backup).await.into_iter().next() else { return DONE };
            let data: Value = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
                Ok(v) => v,
                Err(e) => return Ok(json!({ "ok": false, "error": format!("Not a readable JSON file: {e}") })),
            };
            if data["app"] != "interleaved-study" || !data["colls"].is_object() { return Ok(json!({ "ok": false, "error": "This file is not an Interleaved Study backup." })) }
            let ok = confirm("Replace all current data with this backup?".into(),
                "Your API keys, provider addresses and scheduled task setup stay as they are. Consider exporting a backup of the current data first.".into(), "Replace all data".into(), true).await;
            if !ok { return DONE }
            {
                let mut c = app.lock();
                c.queue_cancel(|_| true);
                // Keep a safety copy of the current data next to the store
                let _ = std::fs::write(app.data_dir.join(format!("before-restore-{}.json", now_ms() as i64)), c.store.dump().to_string());
                // Where keys are sent is this machine's choice, never a backup's.
                let keep = json!({ "ai": { "baseUrls": c.settings_raw["ai"]["baseUrls"], "routine": c.settings_raw["ai"]["routine"] }, "tools": c.settings_raw["tools"] });
                c.store.restore(&data);
                c.settings_raw = merge_settings(Some(&deep_merge(&c.store.kv_get::<Value>("settings").unwrap_or(json!({})), &keep)));
                c.save_settings();
                c.session = c.store.kv_get::<SessionState>("session").filter(|x| x.active);
                c.queue_start();
                c.emit_data();
                c.save_session();
            }
            app.kick();
            Ok(json!({ "ok": true }))
        }
        "testNotification" => {
            let c = app.lock();
            (c.fx.alert)("Interleaved Study", &format!("Notifications work. Sound: {}.", c.settings.notify.sound));
            DONE
        }
        _ => Err(format!("Unknown method {method}")),
    }
}

#[allow(dead_code)]
fn _settings_used() -> &'static str { settings::ROUTINE_MODEL }
