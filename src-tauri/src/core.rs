// The app's state behind one lock, and the domain operations on it: courses, lessons, topics (with text kept
// separately), history carry-over, reviews, overrides, daily stats, snapshots, organize-job creation and estimates.
use crate::history::{carry_history, has_history, match_topics, MatchItem};
use crate::organize::{ambiguous_pairs, cache_key, finalize_windows, topic_excerpts, window_cache_content, ExcerptTopic, WinResult, ORGANIZE_SYSTEM};
use crate::files;
use crate::preprocess::{build_windows, est_tokens, first_sentence_name, provisional_topics, safe_name, slice_text, Window};
use crate::rx;
use crate::sched::{apply_review, mark_solid, mastery, reset_sched, set_due, status, ReviewInput};
use crate::settings::{self, Settings};
use crate::store::Store;
use crate::types::*;
use crate::util::{add_time, clamp, course_code, day_key, jround, natural_cmp, now_ms, round1, suggested_minutes, uid, word_count};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// How the core reaches the outside: window events and system notifications. Tests use the no-op default.
#[derive(Clone)]
pub struct Fx {
    pub emit: Arc<dyn Fn(&str, Value) + Send + Sync>,
    pub alert: Arc<dyn Fn(&str, &str) + Send + Sync>,
}

impl Default for Fx {
    fn default() -> Self {
        Fx { emit: Arc::new(|_, _| {}), alert: Arc::new(|_, _| {}) }
    }
}

pub struct RunningJob {
    pub job: Job,
    pub cancel: CancellationToken,
}

#[derive(Default)]
pub struct QueueState {
    pub user_paused: bool,
    pub rate_limit: Option<RateLimitInfo>,
    pub paused_until: Option<f64>,
    pub pause_reason: Option<String>,
    pub running: Vec<RunningJob>,
    pub looping: bool,
    pub waiters: HashMap<String, oneshot::Sender<Result<Value, String>>>,
    pub run: Option<RunInfo>,
}

pub struct Core {
    pub store: Store,
    pub settings_raw: Value,
    pub settings: Settings,
    pub secrets: HashMap<String, String>,
    pub session: Option<SessionState>,
    pub q: QueueState,
    pub fx: Fx,
    pub last_sheet: Option<String>,
}

pub fn priority(kind: &str) -> i64 {
    match kind { "quiz" | "grade" => 1, "deadlines" => 2, "enrich" | "dedupe" => 4, _ => 5 }
}

pub struct TopicItem {
    pub topic: Topic,
    pub text: String,
    pub excerpt: Option<String>,
}

pub struct NewJob {
    pub kind: &'static str,
    pub doc_id: Option<String>,
    pub topic_id: Option<String>,
    pub label: String,
    pub payload: Value,
    pub est_tokens: f64,
}

#[derive(Default)]
pub struct AddTopic {
    pub doc_id: Option<String>,
    pub course_id: Option<String>,
    pub course_name: Option<String>,
    pub lesson_title: Option<String>,
    pub unit: Option<String>,
    pub name: String,
    pub text: String,
}

impl Core {
    pub fn new(store: Store, fx: Fx) -> Core {
        let raw = settings::merge_settings(store.kv_get::<Value>("settings").as_ref());
        let typed = settings::typed(&raw);
        let session = store.kv_get::<SessionState>("session").filter(|s| s.active);
        let user_paused = store.kv_get::<bool>("queuePaused").unwrap_or(false);
        let rate_limit = if user_paused { store.kv_get::<RateLimitInfo>("rateLimit") } else { None };
        let mut c = Core {
            store, settings_raw: raw, settings: typed, secrets: HashMap::new(), session, fx, last_sheet: None,
            q: QueueState { user_paused, rate_limit, ..Default::default() },
        };
        c.save_settings();
        c
    }

    pub fn now(&self) -> f64 { now_ms() }
    pub fn emit_data(&self) { (self.fx.emit)("data", Value::Null) }

    pub fn save_settings(&mut self) {
        self.settings = settings::typed(&self.settings_raw);
        self.store.kv_set("settings", &self.settings_raw);
    }

    pub fn secret(&self, name: &str) -> Option<&str> {
        self.secrets.get(name).map(|s| s.as_str()).filter(|s| !s.is_empty())
    }

    /// The secret the selected provider needs: its API key, or the scheduled task's trigger token.
    pub fn active_key(&self) -> Option<String> {
        let p = self.settings.ai.provider.as_str();
        let k = self.secret(p)?.to_string();
        if p == "routine" && (self.settings.ai.routine.fire_url.trim().is_empty() || self.settings.ai.routine.repo.trim().is_empty()) { return None }
        Some(k)
    }

    // ---------- courses ----------
    pub fn ensure_course(&self, name: &str) -> Course {
        let clean = if name.trim().is_empty() { "Unsorted" } else { name.trim() };
        let all = self.store.all::<Course>("courses");
        if let Some(c) = all.iter().find(|c| c.name.to_lowercase() == clean.to_lowercase()) { return c.clone() }
        let c = Course { id: uid(), name: clean.to_string(), color: (all.len() % 8) as i64, created_at: self.now() };
        self.store.put("courses", &c);
        c
    }

    /// A course the student names themselves (the New course flow); unlike ensure_course, a taken name is an error.
    pub fn create_course(&self, name: &str) -> Result<Course, String> {
        let clean = name.trim();
        if clean.is_empty() { return Err("Give the course a name.".into()) }
        if self.store.all::<Course>("courses").iter().any(|c| c.name.to_lowercase() == clean.to_lowercase()) {
            return Err(format!("There is already a course called \"{clean}\"."));
        }
        Ok(self.ensure_course(clean))
    }

    /// One-time fix for courses named after the import folder ("Transcriptions"): when every lesson came from that
    /// folder and the file names share one course code, rename the course to the code.
    pub fn name_courses_by_code(&self) {
        if self.store.kv_get::<bool>("coursesNamedByCode").unwrap_or(false) { return }
        self.store.kv_set("coursesNamedByCode", &true);
        let courses = self.store.all::<Course>("courses");
        for c in &courses {
            let docs: Vec<Doc> = self.store.by_parent::<Doc>("docs", &c.id).into_iter().filter(|d| !d.path.is_empty()).collect();
            let folder = |d: &Doc| std::path::Path::new(&d.path).parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if docs.is_empty() || !docs.iter().all(|d| folder(d) == c.name) { continue }
            let found: Vec<String> = docs.iter().filter_map(|d| course_code(&d.file_name)).collect();
            let Some(code) = found.first().cloned() else { continue };
            if found.iter().any(|x| *x != code) || found.len() * 2 < docs.len() { continue }
            if courses.iter().any(|o| o.name.to_lowercase() == code.to_lowercase()) { continue }
            self.store.put("courses", &Course { name: code, ..c.clone() });
        }
    }

    pub fn rename_course(&self, id: &str, name: &str) {
        let Some(c) = self.store.get::<Course>("courses", id) else { return };
        if name.trim().is_empty() { return }
        self.store.put("courses", &Course { name: name.trim().to_string(), ..c });
    }

    pub fn delete_course(&self, id: &str) {
        self.store.tx(|| {
            for d in self.store.by_parent::<Doc>("docs", id) { self.delete_doc(&d.id); }
            for t in self.store.all::<Topic>("topics").into_iter().filter(|t| t.course_id == id) { self.delete_topic(&t.id); }
            for mut dl in self.store.all::<Deadline>("deadlines").into_iter().filter(|x| x.course_id.as_deref() == Some(id)) {
                dl.course_id = None;
                self.store.put("deadlines", &dl);
            }
            self.store.del("courses", id);
        })
    }

    // ---------- docs ----------
    pub fn doc_prep(&self, doc_id: &str) -> Option<DocPrep> {
        serde_json::from_str(&self.store.get_text(&format!("doc:{doc_id}"))?).ok()
    }

    pub fn save_doc_prep(&self, doc_id: &str, prep: &DocPrep) {
        self.store.put_text(&format!("doc:{doc_id}"), &serde_json::to_string(prep).unwrap_or_default());
    }

    pub fn rename_doc(&self, id: &str, title: &str) {
        let Some(mut d) = self.store.get::<Doc>("docs", id) else { return };
        if title.trim().is_empty() { return }
        d.title = title.trim().to_string();
        d.title_locked = Some(true);
        d.updated_at = self.now();
        self.store.put("docs", &d);
    }

    pub fn delete_doc(&self, id: &str) {
        self.store.tx(|| {
            for t in self.store.by_parent::<Topic>("topics", id) { self.delete_topic(&t.id); }
            for j in self.store.by_parent::<Job>("jobs", id) { self.store.del("jobs", &j.id); }
            for w in self.store.by_parent::<Value>("wins", id) { if let Some(wid) = w["id"].as_str() { self.store.del("wins", wid); } }
            for mut dl in self.store.all::<Deadline>("deadlines").into_iter().filter(|x| x.doc_id.as_deref() == Some(id)) {
                dl.doc_id = None;
                self.store.put("deadlines", &dl);
            }
            self.store.del_text(&format!("doc:{id}"));
            self.store.del("docs", id);
        })
    }

    pub fn history_count(&self, doc_id: &str) -> usize {
        self.store.by_parent::<Topic>("topics", doc_id).iter().filter(|t| has_history(t)).count()
    }

    // ---------- topics ----------
    pub fn new_topic(&self, doc_id: &str, course_id: &str, name: &str) -> Topic {
        Topic { id: uid(), doc_id: doc_id.into(), course_id: course_id.into(), name: name.into(), minutes: 10.0, created_at: self.now(), ..Default::default() }
    }

    pub fn topic_text(&self, id: &str) -> String {
        self.store.get_text(&format!("topic:{id}")).unwrap_or_default()
    }

    /// The topic's verbatim section of the lesson as the AI cut it, or None when there is none.
    pub fn topic_excerpt(&self, id: &str) -> Option<String> {
        self.store.get_text(&format!("excerpt:{id}"))
    }

    pub fn set_excerpt(&self, id: &str, excerpt: Option<&str>) {
        match excerpt {
            Some(e) if !e.is_empty() => self.store.put_text(&format!("excerpt:{id}"), e),
            _ => self.store.del_text(&format!("excerpt:{id}")),
        }
    }

    pub fn save_topic(&self, t: &Topic, text: Option<&str>) {
        self.store.put("topics", t);
        if let Some(x) = text { self.store.put_text(&format!("topic:{}", t.id), x); }
        let body_text = match text { Some(x) => x.to_string(), None => self.topic_text(&t.id) };
        self.store.index(&t.id, &t.name, &format!("{}\n{}\n{}\n{}", t.summary, t.terms.join(" "), t.unit, body_text));
    }

    pub fn delete_topic(&self, id: &str) {
        self.store.del("topics", id);
        self.store.del_text(&format!("topic:{id}"));
        self.store.del_text(&format!("excerpt:{id}"));
        self.store.del("quizzes", id);
        self.store.unindex(id);
    }

    pub fn rename_topic(&self, id: &str, name: &str) {
        let Some(mut t) = self.store.get::<Topic>("topics", id) else { return };
        let n = name.trim();
        if n.is_empty() { return }
        t.name = safe_name(n, &self.topic_text(id), n);
        t.provisional = false;
        self.save_topic(&t, None);
    }

    pub fn update_topic(&self, id: &str, patch: &Value) {
        let Some(mut t) = self.store.get::<Topic>("topics", id) else { return };
        if let Some(n) = patch.get("notes").and_then(|x| x.as_str()) { t.notes = n.to_string(); }
        if let Some(u) = patch.get("unit").and_then(|x| x.as_str()) { if !u.trim().is_empty() { t.unit = u.trim().to_string(); } }
        if let Some(m) = patch.get("minutes").and_then(|x| x.as_f64()) { if m.is_finite() { t.minutes = clamp(jround(m), 1.0, 240.0); } }
        self.save_topic(&t, None);
    }

    pub fn renumber(&self, doc_id: &str) {
        let mut ts = self.store.by_parent::<Topic>("topics", doc_id);
        ts.sort_by(|a, b| a.ord.partial_cmp(&b.ord).unwrap_or(std::cmp::Ordering::Equal));
        for (i, mut t) in ts.into_iter().enumerate() {
            if t.ord != i as f64 { t.ord = i as f64; self.store.put("topics", &t); }
        }
    }

    /// Replace a lesson's topics, carrying history over to new topics that match old ones by name or text.
    /// Returns how many were carried and how many old topics with history could not be matched.
    pub fn replace_doc_topics(&self, doc_id: &str, items: Vec<TopicItem>) -> (usize, usize) {
        let olds = self.store.by_parent::<Topic>("topics", doc_id);
        let old_items: Vec<MatchItem> = olds.iter().map(|o| MatchItem { id: o.id.clone(), name: o.name.clone(), text: self.topic_text(&o.id) }).collect();
        let new_items: Vec<MatchItem> = items.iter().map(|i| MatchItem { id: i.topic.id.clone(), name: i.topic.name.clone(), text: i.text.clone() }).collect();
        let matched = match_topics(&old_items, &new_items);
        let old_by_id: HashMap<&str, &Topic> = olds.iter().map(|o| (o.id.as_str(), o)).collect();
        let mut carried = 0;
        let mut used_old: HashSet<String> = HashSet::new();
        self.store.tx(|| {
            for o in &olds { self.delete_topic(&o.id); }
            for it in &items {
                let mut t = it.topic.clone();
                if let Some(o) = matched.get(&it.topic.id).and_then(|id| old_by_id.get(id.as_str())) {
                    used_old.insert(o.id.clone());
                    if has_history(o) { carried += 1; }
                    t = carry_history(&t, o);
                    t.id = o.id.clone(); // keep the id so deadlines and quizzes stay linked
                }
                self.save_topic(&t, Some(&it.text));
                self.set_excerpt(&t.id, it.excerpt.as_deref());
            }
        });
        let dropped = olds.iter().filter(|o| !used_old.contains(&o.id) && has_history(o)).count();
        (carried, dropped)
    }

    /// Provisional topics (heading or first-sentence names, never "Part N") for a lesson.
    pub fn provisional_items(&self, doc: &Doc, prep: &DocPrep) -> Vec<TopicItem> {
        let pace = self.settings.study.pace;
        provisional_topics(&prep.paras, &doc.title).into_iter().enumerate().map(|(i, p)| {
            let mut t = self.new_topic(&doc.id, &doc.course_id, &p.name);
            t.unit = p.unit;
            t.ord = i as f64;
            t.start = Some(p.start);
            t.end = Some(p.end);
            t.words = p.words;
            t.minutes = suggested_minutes(p.words, None, pace);
            t.provisional = true;
            TopicItem { topic: t, text: slice_text(&prep.paras, p.start, p.end), excerpt: None }
        }).collect()
    }

    pub fn merge_topics(&self, ids: &[String], name: Option<&str>) -> Result<String, String> {
        let ts: Vec<Topic> = ids.iter().filter_map(|id| self.store.get::<Topic>("topics", id)).collect();
        if ts.len() < 2 { return Err("Select at least two topics to merge".into()) }
        let mut by_weight: Vec<&Topic> = ts.iter().collect();
        by_weight.sort_by(|a, b| b.reviews.len().cmp(&a.reviews.len())
            .then(b.spent.partial_cmp(&a.spent).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.ord.partial_cmp(&b.ord).unwrap_or(std::cmp::Ordering::Equal)));
        let base = by_weight[0].clone();
        let mut ordered: Vec<&Topic> = ts.iter().collect();
        ordered.sort_by(|a, b| if a.doc_id == b.doc_id { a.ord.partial_cmp(&b.ord).unwrap_or(std::cmp::Ordering::Equal) } else { a.doc_id.cmp(&b.doc_id) });
        let text = ordered.iter().map(|t| self.topic_text(&t.id)).filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n\n");
        let excerpts: Vec<Option<String>> = ordered.iter().map(|t| self.topic_excerpt(&t.id)).collect();
        let same_doc = ts.iter().all(|t| t.doc_id == base.doc_id);
        let words = word_count(&text);
        let uniq = |it: Vec<String>, max: usize| { let mut seen = HashSet::new(); it.into_iter().filter(|x| seen.insert(x.clone())).take(max).collect::<Vec<_>>() };
        let mut merged = base.clone();
        if let Some(n) = name.filter(|n| !n.trim().is_empty()) { merged.name = safe_name(n, &text, &base.name); }
        merged.start = if same_doc && ts.iter().all(|t| t.start.is_some()) { ts.iter().filter_map(|t| t.start).min() } else { None };
        merged.end = if same_doc && ts.iter().all(|t| t.end.is_some()) { ts.iter().filter_map(|t| t.end).max() } else { None };
        merged.words = words;
        merged.minutes = suggested_minutes(words, base.difficulty, self.settings.study.pace);
        merged.spent = ts.iter().map(|t| t.spent).sum();
        merged.terms = uniq(ordered.iter().flat_map(|t| t.terms.clone()).collect(), 6);
        merged.prereqs = uniq(ordered.iter().flat_map(|t| t.prereqs.clone()).collect(), 3);
        if base.summary.is_empty() { merged.summary = ordered.iter().map(|t| t.summary.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" "); }
        merged.notes = ordered.iter().map(|t| t.notes.as_str()).filter(|n| !n.trim().is_empty()).collect::<Vec<_>>().join("\n\n");
        merged.ord = ts.iter().filter(|t| t.doc_id == base.doc_id).map(|t| t.ord).fold(f64::INFINITY, f64::min);
        merged.provisional = false;
        self.store.tx(|| {
            for t in &ts { if t.id != base.id { self.delete_topic(&t.id); } }
            self.store.del("quizzes", &base.id);
            self.save_topic(&merged, Some(&text));
            let joined = if excerpts.iter().all(|e| e.as_deref().is_some_and(|x| !x.is_empty())) { Some(excerpts.iter().flatten().cloned().collect::<Vec<_>>().join("\n\n")) } else { None };
            self.set_excerpt(&base.id, joined.as_deref());
            self.renumber(&base.doc_id);
        });
        Ok(base.id)
    }

    pub fn split_topic(&self, id: &str, at: usize, name: Option<&str>) -> Result<String, String> {
        let t = self.store.get::<Topic>("topics", id).ok_or("Topic not found")?;
        let text = self.topic_text(id);
        let paras: Vec<&str> = rx!(r"\n{2,}").split(&text).collect();
        if at < 1 || at >= paras.len() { return Err("Choose a split point between two paragraphs".into()) }
        let a_text = paras[..at].join("\n\n");
        let b_text = paras[at..].join("\n\n");
        let pace = self.settings.study.pace;
        let ranged = matches!((t.start, t.end), (Some(s), Some(e)) if (e - s + 1) as usize == paras.len());
        let (a_words, b_words) = (word_count(&a_text), word_count(&b_text));
        let mut a = t.clone();
        a.words = a_words;
        a.minutes = suggested_minutes(a_words, t.difficulty, pace);
        if ranged { a.end = Some(t.start.unwrap() + at as i64 - 1); } else { a.start = None; a.end = None; }
        let cand = match name.filter(|n| !n.is_empty()) { Some(n) => n.to_string(), None => first_sentence_name(&b_text) };
        let mut b = self.new_topic(&t.doc_id, &t.course_id, &safe_name(&cand, &b_text, &t.name));
        b.unit = t.unit.clone();
        b.ord = t.ord + 0.5;
        b.start = if ranged { Some(t.start.unwrap() + at as i64) } else { None };
        b.end = if ranged { t.end } else { None };
        b.words = b_words;
        b.minutes = suggested_minutes(b_words, t.difficulty, pace);
        b.difficulty = t.difficulty;
        self.store.tx(|| {
            self.store.del("quizzes", &t.id);
            self.set_excerpt(&t.id, None); // the AI's cut no longer fits either half; their text is shown instead
            self.save_topic(&a, Some(&a_text));
            self.save_topic(&b, Some(&b_text));
            self.renumber(&t.doc_id);
        });
        Ok(b.id)
    }

    pub fn add_topic(&self, input: &AddTopic) -> Result<Topic, String> {
        let mut doc = input.doc_id.as_deref().filter(|d| !d.is_empty()).and_then(|d| self.store.get::<Doc>("docs", d));
        if doc.is_none() {
            let course = match input.course_id.as_deref().filter(|c| !c.is_empty()) {
                Some(cid) => self.store.get::<Course>("courses", cid).ok_or("Course not found")?,
                None => self.ensure_course(input.course_name.as_deref().unwrap_or("Unsorted")),
            };
            let title = input.lesson_title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("Manual topics").trim().to_string();
            doc = self.store.by_parent::<Doc>("docs", &course.id).into_iter().find(|d| d.title.to_lowercase() == title.to_lowercase());
            if doc.is_none() {
                let d = Doc { id: uid(), course_id: course.id.clone(), title, title_locked: Some(true), ext: "manual".into(), status: "organized".into(), created_at: self.now(), updated_at: self.now(), ..Default::default() };
                self.store.put("docs", &d);
                doc = Some(d);
            }
        }
        let doc = doc.unwrap();
        let words = word_count(&input.text);
        let mut t = self.new_topic(&doc.id, &doc.course_id, &safe_name(&input.name, &input.text, &doc.title));
        t.unit = input.unit.as_deref().map(|u| u.trim()).filter(|u| !u.is_empty()).unwrap_or(&doc.title).to_string();
        t.ord = self.store.count_parent("topics", &doc.id) as f64;
        t.words = words;
        t.minutes = suggested_minutes(words, None, self.settings.study.pace);
        t.manual = Some(true);
        self.save_topic(&t, Some(input.text.trim()));
        Ok(t)
    }

    // ---------- reviews and overrides ----------
    pub fn review(&self, topic_id: &str, r: &ReviewInput, count_daily: bool) {
        let Some(t) = self.store.get::<Topic>("topics", topic_id) else { return };
        self.store.put("topics", &apply_review(&t, r));
        if count_daily { self.add_daily(r.minutes, if r.rating.is_some() { 1.0 } else { 0.0 }, r.at); }
    }

    pub fn add_daily(&self, minutes: f64, topics: f64, at: f64) {
        let id = day_key(at);
        let mut d = self.store.get::<Daily>("daily", &id).unwrap_or(Daily { id: id.clone(), ..Default::default() });
        d.minutes = round1(d.minutes + minutes.max(0.0));
        d.topics += topics;
        self.store.put("daily", &d);
    }

    pub fn set_day_goal(&self, goal: Option<DayGoal>) {
        let id = day_key(self.now());
        let mut d = self.store.get::<Daily>("daily", &id).unwrap_or(Daily { id: id.clone(), ..Default::default() });
        d.goal = goal;
        self.store.put("daily", &d);
    }

    pub fn override_topic(&self, id: &str, a: &Value) {
        let Some(t) = self.store.get::<Topic>("topics", id) else { return };
        let now = self.now();
        let num = |k: &str| a.get(k).and_then(|x| x.as_f64());
        let n = match a.get("kind").and_then(|x| x.as_str()).unwrap_or("") {
            "due" => set_due(&t, num("at").unwrap_or(now), now),
            "dueNow" => set_due(&t, now, now),
            "push" => set_due(&t, add_time(now.max(t.due.unwrap_or(now)), num("amount").unwrap_or(0.0), a.get("unit").and_then(|x| x.as_str()).unwrap_or("days")), now),
            "solid" => mark_solid(&t, now),
            "reset" => reset_sched(&t),
            "score" => { let mut n = t.clone(); n.override_score = Some(clamp(jround(num("value").unwrap_or(0.0)), 0.0, 100.0)); n }
            "clearScore" => { let mut n = t.clone(); n.override_score = None; n }
            _ => return,
        };
        self.store.put("topics", &n);
    }

    pub fn snapshot_today(&self) -> bool {
        let id = day_key(self.now());
        if self.store.has("snapshots", &id) { return false }
        let now = self.now();
        let ts = self.store.all::<Topic>("topics");
        let mut s = Snapshot { id, total: ts.len() as i64, ..Default::default() };
        let mut sum = 0.0;
        for t in &ts {
            sum += mastery(t, now);
            match status(t, now) { "solid" => s.solid += 1, "progress" => s.progress += 1, "review" => s.review += 1, _ => s.fresh += 1 }
        }
        s.avg = if ts.is_empty() { 0.0 } else { jround(sum / ts.len() as f64) };
        self.store.put("snapshots", &s);
        true
    }

    // ---------- organize jobs ----------
    pub fn create_job(&self, j: NewJob) -> Job {
        let mut job = Job {
            id: uid(), kind: j.kind.into(), doc_id: j.doc_id, topic_id: j.topic_id, payload: j.payload, status: "queued".into(), attempts: 0,
            next_at: 0.0, error: None, est_tokens: j.est_tokens, priority: priority(j.kind), created_at: self.now(), label: j.label,
        };
        // Strict creation order even within the same millisecond
        let last = self.store.kv_get::<f64>("jobSeq").unwrap_or(0.0);
        job.created_at = job.created_at.max(last + 1.0);
        self.store.kv_set("jobSeq", &job.created_at);
        self.store.put("jobs", &job);
        job
    }

    pub fn windows_for(&self, doc_id: &str) -> Option<(DocPrep, Vec<Window>)> {
        let prep = self.doc_prep(doc_id)?;
        if prep.paras.is_empty() { return None }
        let with_file = self.store.get::<Doc>("docs", doc_id).is_some_and(|d| self.file_plan(&d).is_some());
        // Every request of a lesson sent with its file carries the whole file, so fewer, larger windows cost less.
        let wins = build_windows(&prep.paras, self.settings.ai.window() * if with_file { 3.0 } else { 1.0 });
        Some((prep, wins))
    }

    /// The MIME type this lesson's original file goes to the current model as, if it does.
    pub fn file_plan(&self, doc: &Doc) -> Option<&'static str> {
        files::plan(&self.settings.ai, &self.caps(&self.settings.ai.model), doc)
    }

    /// Cache key of one organize window; a window sent with the file is a different request from the text alone.
    pub fn organize_key(&self, doc: &Doc, prep: &DocPrep, from: i64, to: i64, first: bool, with_file: bool) -> String {
        let ai = &self.settings.ai;
        let mut content = window_cache_content(&prep.outline, &prep.paras, from, to, first);
        if let (true, Some(h)) = (with_file, &doc.file_hash) { content.push_str(&format!("\nfile:{h}")); }
        cache_key("organize", &content, &ai.model, ai.include_model_in_cache_key)
    }

    /// Output tokens to allow when the AI writes out a whole file.
    pub fn read_output(doc: &Doc) -> f64 {
        if files::is_image(&doc.ext) { 4000.0 } else { (doc.pages.unwrap_or(10).max(1).min(files::READ_MAX_PAGES) as f64 * 900.0).clamp(4000.0, 40000.0) }
    }

    pub fn window_cost(&self, prep: &DocPrep, tokens: f64) -> f64 {
        tokens + est_tokens(ORGANIZE_SYSTEM.chars().count() + prep.outline.chars().count() + 300)
    }

    pub fn enqueue_organize(&self, doc_id: &str, fresh: bool) -> usize {
        let Some(mut doc) = self.store.get::<Doc>("docs", doc_id) else { return 0 };
        // No text the Mac could read: the AI reads the file first, then the lesson is organized as usual.
        if doc.needs_reading == Some(true) {
            if self.file_plan(&doc).is_none() {
                let ai = &self.settings.ai;
                doc.status = "error".into();
                doc.error = Some(files::plan_problem(ai, &self.caps(&ai.model), &doc).unwrap_or_else(|| format!(
                    "The Mac found no text in this file, and {} cannot read files. Switch to Gemini or the Claude scheduled task in Settings, then retry.", files::provider_name(ai))));
                self.store.put("docs", &doc);
                return 0;
            }
            self.store.tx(|| {
                for j in self.store.by_parent::<Job>("jobs", doc_id) { if (j.kind == "organize" || j.kind == "read") && j.status != "running" { self.store.del("jobs", &j.id); } }
                self.create_job(NewJob {
                    kind: "read", doc_id: Some(doc_id.into()), topic_id: None, label: format!("Read \"{}\"", doc.title),
                    payload: json!({ "fresh": fresh }), est_tokens: files::file_tokens(&doc) + Self::read_output(&doc),
                });
                doc.status = "queued".into();
                doc.windows = 0;
                doc.wins_done = 0;
                doc.error = None;
                doc.updated_at = self.now();
                self.store.put("docs", &doc);
            });
            return 1;
        }
        let Some((prep, wins)) = self.windows_for(doc_id) else {
            doc.status = "error".into();
            doc.error = Some("No stored text for this lesson. Re-import the file.".into());
            self.store.put("docs", &doc);
            return 0;
        };
        let n = wins.len();
        let with_file = self.file_plan(&doc).is_some();
        self.store.tx(|| {
            for j in self.store.by_parent::<Job>("jobs", doc_id) { if (j.kind == "organize" || j.kind == "read") && j.status != "running" { self.store.del("jobs", &j.id); } }
            for w in self.store.by_parent::<Value>("wins", doc_id) { if let Some(wid) = w["id"].as_str() { self.store.del("wins", wid); } }
            for w in &wins {
                self.create_job(NewJob {
                    kind: "organize", doc_id: Some(doc_id.into()), topic_id: None, label: format!("Organize \"{}\" ({}/{})", doc.title, w.index + 1, n),
                    payload: json!({ "win": w.index, "from": w.from, "to": w.to, "of": n, "fresh": fresh }),
                    est_tokens: self.window_cost(&prep, w.tokens) + if with_file { files::file_tokens(&doc) } else { 0.0 },
                });
            }
            doc.status = "queued".into();
            doc.windows = n as i64;
            doc.wins_done = 0;
            doc.error = None;
            doc.updated_at = self.now();
            self.store.put("docs", &doc);
        });
        n
    }

    pub fn estimate(&self, doc_ids: Option<&[String]>) -> Estimate {
        let ai = &self.settings.ai;
        let docs: Vec<Doc> = self.store.all::<Doc>("docs").into_iter().filter(|d| match doc_ids { Some(ids) => ids.contains(&d.id), None => d.status == "awaiting" || d.status == "needs" }).collect();
        let (mut requests, mut cached, mut tokens) = (0, 0, 0.0);
        for d in &docs {
            let with_file = self.file_plan(d).is_some();
            let file = if with_file { files::file_tokens(d) } else { 0.0 };
            if d.needs_reading == Some(true) {
                if !with_file { continue }
                // Reading comes first; the organize windows after it are not known yet.
                requests += 1;
                tokens += file + Self::read_output(d);
                continue;
            }
            let Some((prep, wins)) = self.windows_for(&d.id) else { continue };
            for w in &wins {
                let key = self.organize_key(d, &prep, w.from, w.to, w.index == 0, with_file);
                if self.store.has("cache", &key) { cached += 1; continue }
                requests += 1;
                tokens += self.window_cost(&prep, w.tokens) + ai.max_output_tokens * if with_file { 3.0 } else { 1.0 } + file;
            }
        }
        Estimate { docs: docs.len(), requests, cached, tokens }
    }

    /// Organizing was cancelled: partial window results go; the lesson keeps the topics it had and can be organized again.
    pub fn cancel_organize(&self, doc_id: &str) {
        let Some(mut doc) = self.store.get::<Doc>("docs", doc_id) else { return };
        self.store.tx(|| {
            for w in self.store.by_parent::<Value>("wins", doc_id) { if let Some(wid) = w["id"].as_str() { self.store.del("wins", wid); } }
            let organized = self.store.by_parent::<Topic>("topics", doc_id).iter().any(|t| !t.provisional);
            doc.status = if organized { "organized" } else { "awaiting" }.into();
            doc.wins_done = 0;
            doc.error = None;
            doc.updated_at = self.now();
            self.store.put("docs", &doc);
        })
    }

    /// All windows of a lesson are done: build final topics (with history carry-over) and queue ambiguous merges.
    pub fn finalize_doc(&self, doc_id: &str) {
        let (Some(mut doc), Some(prep)) = (self.store.get::<Doc>("docs", doc_id), self.doc_prep(doc_id)) else { return };
        let mut wins: Vec<(String, i64, WinResult)> = self.store.by_parent::<Value>("wins", doc_id).into_iter().filter_map(|w| {
            Some((w["id"].as_str()?.to_string(), w["idx"].as_i64()?, serde_json::from_value::<WinResult>(w["result"].clone()).ok()?))
        }).collect();
        wins.sort_by_key(|w| w.1);
        let results: Vec<WinResult> = wins.iter().map(|w| w.2.clone()).collect();
        let fin = finalize_windows(&results, &prep.paras, &doc.title);
        let pace = self.settings.study.pace;
        let title = if doc.title_locked == Some(true) { None } else { results.first().and_then(|r| r.title.clone()).filter(|t| !t.is_empty()) };
        let ex_in: Vec<ExcerptTopic> = fin.iter().map(|f| ExcerptTopic { start: f.t.start, end: f.t.end, from: &f.t.from }).collect();
        let excerpts = topic_excerpts(&prep.paras, &ex_in);
        let items: Vec<TopicItem> = fin.iter().enumerate().map(|(i, f)| {
            let mut t = self.new_topic(doc_id, &doc.course_id, &f.t.name);
            t.unit = f.t.unit.clone();
            t.ord = i as f64;
            t.start = Some(f.t.start);
            t.end = Some(f.t.end);
            t.words = f.words;
            t.minutes = suggested_minutes(f.words, Some(f.t.difficulty), pace);
            t.summary = f.t.summary.clone();
            t.terms = f.t.terms.clone();
            t.prereqs = f.t.prereqs.clone();
            t.difficulty = Some(f.t.difficulty);
            TopicItem { topic: t, text: slice_text(&prep.paras, f.t.start, f.t.end), excerpt: excerpts.get(i).cloned().filter(|e| !e.is_empty()) }
        }).collect();
        let (_, dropped) = self.replace_doc_topics(doc_id, items);
        self.store.tx(|| {
            for w in &wins { self.store.del("wins", &w.0); }
            if let Some(t) = &title { doc.title = t.clone(); }
            doc.status = "organized".into();
            doc.wins_done = doc.windows;
            doc.error = None;
            doc.updated_at = self.now();
            if dropped > 0 { doc.import_note = Some(format!("{dropped} earlier topic(s) with study history had no match after organizing.")); }
            self.store.put("docs", &doc);
        });
        let mut topics = self.store.by_parent::<Topic>("topics", doc_id);
        topics.sort_by(|a, b| a.ord.partial_cmp(&b.ord).unwrap_or(std::cmp::Ordering::Equal));
        let pairs = ambiguous_pairs(&topics.iter().map(|t| (t.name.clone(), t.unit.clone())).collect::<Vec<_>>());
        if !pairs.is_empty() {
            self.create_job(NewJob {
                kind: "dedupe", doc_id: Some(doc_id.into()), topic_id: None, label: format!("Check similar topics in \"{}\"", doc.title),
                payload: json!({ "pairs": pairs.iter().map(|(a, b)| json!([topics[*a].id, topics[*b].id])).collect::<Vec<_>>() }),
                est_tokens: 200.0 + pairs.len() as f64 * 80.0,
            });
        }
    }

    pub fn data_snapshot(&self) -> Value {
        let mut courses = self.store.all::<Course>("courses");
        courses.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let mut docs = self.store.all::<Doc>("docs");
        docs.sort_by(|a, b| natural_cmp(&a.title, &b.title));
        let mut deadlines = self.store.all::<Deadline>("deadlines");
        deadlines.sort_by(|a, b| a.date.cmp(&b.date));
        let mut daily = self.store.all::<Daily>("daily");
        daily.sort_by(|a, b| a.id.cmp(&b.id));
        let mut snapshots = self.store.all::<Snapshot>("snapshots");
        snapshots.sort_by(|a, b| a.id.cmp(&b.id));
        json!({ "courses": courses, "docs": docs, "topics": self.store.all::<Topic>("topics"), "deadlines": deadlines, "daily": daily, "snapshots": snapshots })
    }

    // ---------- deadlines ----------
    pub fn course_name(&self, id: Option<&str>) -> Option<String> {
        self.store.get::<Course>("courses", id?).map(|c| c.name)
    }

    pub fn add_deadline(&self, d: &DeadlineInput) -> bool {
        let mut course_id = d.course_id.clone().filter(|c| !c.is_empty());
        let cname = d.course_name.clone().filter(|c| !c.is_empty());
        if course_id.is_none() {
            if let Some(n) = &cname { course_id = self.store.all::<Course>("courses").into_iter().find(|c| c.name.to_lowercase() == n.to_lowercase()).map(|c| c.id); }
        }
        let key = crate::deadlines::deadline_key(&d.title, &d.date, cname.clone().or_else(|| self.course_name(course_id.as_deref())).as_deref());
        if self.store.all::<Deadline>("deadlines").iter().any(|x| x.key == key) { return false }
        let mut notes = d.notes.clone().unwrap_or_default();
        if let (Some(n), None) = (&cname, &course_id) { notes = format!("{notes}{}Course: {n}", if notes.is_empty() { "" } else { "\n" }); }
        self.store.put("deadlines", &Deadline {
            id: uid(), title: d.title.trim().to_string(), date: d.date.clone(), course_id, doc_id: d.doc_id.clone(), topic_ids: d.topic_ids.clone().unwrap_or_default(),
            notes, key, created_at: self.now(),
        });
        true
    }
}
