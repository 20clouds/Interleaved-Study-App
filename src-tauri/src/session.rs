// Study session engine. Everything is computed from timestamps and saved on every change, so a hidden window,
// sleep, restart or crash never loses a session.
use crate::core::Core;
use crate::sched::{apply_review, rank, PickCtx, PickDeadline, ReviewInput};
use crate::settings::PomoSettings;
use crate::types::*;
use crate::util::{parse_day_key, round1, uid};
use serde_json::Value;
use std::collections::HashMap;

pub fn pomodoro_plan(s: &PomoSettings) -> Vec<f64> {
    match s.per_topic.as_str() {
        "n" => vec![s.work; (s.n.round() as usize).max(1)],
        "minutes" => {
            let total = s.minutes.max(1.0);
            let k = ((total / s.work).ceil() as usize).max(1);
            let mut plan = vec![s.work; k];
            plan[k - 1] = round1(total - (k as f64 - 1.0) * s.work);
            plan.into_iter().filter(|m| *m > 0.0).collect()
        }
        _ => vec![s.work],
    }
}

fn review_of(r: &PendingReview) -> ReviewInput {
    ReviewInput { rating: r.rating, quiz: r.quiz, minutes: r.minutes, note: r.note.clone(), at: r.at }
}

impl Core {
    pub fn save_session(&self) {
        match &self.session {
            Some(s) => self.store.kv_set("session", s),
            None => self.store.kv_del("session"),
        }
        (self.fx.emit)("session", serde_json::to_value(&self.session).unwrap_or(Value::Null));
    }

    pub fn remaining_ms(&self) -> f64 {
        match &self.session {
            Some(s) if s.phase == "work" || s.phase == "break" => (s.end_at - s.paused_at.unwrap_or_else(|| self.now())).max(0.0),
            _ => 0.0,
        }
    }

    pub fn pool_topics(&self, pool: &Pool, overlay: Option<&[PendingReview]>) -> Vec<Topic> {
        let all = self.store.all::<Topic>("topics");
        let mut ts: Vec<Topic> = match pool {
            Pool::Topic { ids } | Pool::Sandbox { ids } => all.into_iter().filter(|t| ids.contains(&t.id)).collect(),
            Pool::Unit { doc_id, unit } => all.into_iter().filter(|t| &t.doc_id == doc_id && &t.unit == unit).collect(),
            Pool::Lesson { doc_id } => all.into_iter().filter(|t| &t.doc_id == doc_id).collect(),
            Pool::Course { course_ids } => all.into_iter().filter(|t| course_ids.contains(&t.course_id)).collect(),
            Pool::Global => all,
        };
        if let Some(revs) = overlay.filter(|r| !r.is_empty()) {
            let idx: HashMap<String, usize> = ts.iter().enumerate().map(|(i, t)| (t.id.clone(), i)).collect();
            for r in revs {
                if let Some(&i) = idx.get(&r.topic_id) { ts[i] = apply_review(&ts[i], &review_of(r)); }
            }
        }
        ts
    }

    pub fn pick_deadlines(&self) -> Vec<PickDeadline> {
        self.store.all::<Deadline>("deadlines").into_iter().map(|x| PickDeadline {
            title: x.title, date: parse_day_key(&x.date) + 12.0 * 3_600_000.0, course_id: x.course_id, doc_id: x.doc_id, topic_ids: x.topic_ids,
        }).collect()
    }

    pub fn suggest(&self, pool: &Pool, sandbox: bool, limit: usize) -> Vec<Suggestion> {
        let s = self.session.as_ref();
        let overlay = if sandbox { s.filter(|s| s.sandbox).map(|s| s.sandbox_reviews.as_slice()) } else { None };
        let ctx = PickCtx {
            now: self.now(), last_topic_id: s.and_then(|s| s.last_topic_id.clone()), last_doc_id: s.and_then(|s| s.last_doc_id.clone()),
            last_was_review: s.map(|s| s.last_was_review).unwrap_or(false), deadlines: self.pick_deadlines(),
        };
        let mut r = rank(&self.pool_topics(pool, overlay), &ctx);
        r.truncate(limit);
        r
    }

    pub fn session_start(&mut self, mode: &str, pool: Pool, pool_label: String, topic_id: Option<String>) -> Result<SessionState, String> {
        // A suggestion picked from a preview can be merged or replaced before the click lands.
        if let Some(id) = &topic_id {
            if !self.store.has("topics", id) { return Err("That topic no longer exists (it was merged or reorganized). Pick it again from the updated list.".into()) }
        }
        let sandbox = matches!(pool, Pool::Sandbox { .. });
        self.session = Some(SessionState {
            active: true, id: uid(), mode: if mode == "pomodoro" { "pomodoro" } else { "timed" }.into(), pool: pool.clone(), pool_label, sandbox,
            phase: "next".into(), break_kind: None, topic_id: None, topic_name: String::new(), course_id: None, end_at: 0.0, paused_at: None,
            duration_ms: 0.0, work_ms: 0.0, block_index: 0, blocks_for_topic: 0, block_plan: vec![], cycles: 0, blocks: vec![], proposal: None,
            passed: vec![], last_topic_id: None, last_doc_id: None, last_was_review: false, sandbox_reviews: vec![], message: None,
        });
        let first = topic_id.or_else(|| self.suggest(&pool, sandbox, 1).first().map(|s| s.topic_id.clone()));
        let Some(first) = first else {
            self.session = None;
            return Err("There are no topics in this pool yet.".into());
        };
        self.begin_topic(&first);
        Ok(self.session.clone().expect("session just started"))
    }

    fn begin_topic(&mut self, topic_id: &str) {
        let now = self.now();
        let t = self.store.get::<Topic>("topics", topic_id);
        let pomo = pomodoro_plan(&self.settings.pomo);
        let study = self.settings.study.clone();
        let Some(s) = self.session.as_mut() else { return };
        let Some(t) = t else {
            s.message = Some("That topic no longer exists.".into());
            self.save_session();
            return;
        };
        let plan = if s.mode == "pomodoro" { pomo } else { vec![if study.use_suggested { t.minutes } else { study.minutes }] };
        s.topic_id = Some(t.id.clone());
        s.topic_name = t.name.clone();
        s.course_id = Some(t.course_id.clone());
        s.blocks_for_topic = plan.len();
        s.block_index = 0;
        s.work_ms = 0.0;
        s.phase = "work".into();
        s.break_kind = None;
        s.paused_at = None;
        s.duration_ms = (plan[0] * 60000.0).round();
        s.end_at = now + s.duration_ms;
        s.block_plan = plan;
        s.proposal = None;
        s.passed = vec![];
        s.message = None;
        s.blocks.retain(|b| b.state != "current");
        s.blocks.push(RibbonBlock { topic_id: t.id.clone(), name: t.name, course_id: t.course_id, state: "current".into(), rating: None, minutes: 0.0 });
        self.save_session();
    }

    /// Called every second and after wake from sleep.
    pub fn tick(&mut self) {
        let now = self.now();
        let Some(s) = &self.session else { return };
        if s.paused_at.is_some() { return }
        if s.phase == "work" && now >= s.end_at { self.complete_work(false) } else if s.phase == "break" && now >= s.end_at { self.complete_break() }
    }

    fn elapsed_ms(&self) -> f64 {
        self.session.as_ref().map(|s| (s.duration_ms - self.remaining_ms()).max(0.0)).unwrap_or(0.0)
    }

    fn complete_work(&mut self, early: bool) {
        let elapsed = self.elapsed_ms();
        let Some(s) = self.session.as_mut() else { return };
        s.work_ms += if early { elapsed } else { s.duration_ms };
        s.paused_at = None;
        let pomo = s.mode == "pomodoro" && !early;
        if pomo { s.cycles += 1; }
        if pomo && s.block_index + 1 < s.block_plan.len() {
            self.start_break();
            let s = self.session.as_ref().expect("session");
            let long = if s.break_kind.as_deref() == Some("long") { "long " } else { "" };
            (self.fx.alert)("Work block done", &format!("Take a {long}break. {} continues after it.", s.topic_name));
            return;
        }
        s.phase = "rate".into();
        s.end_at = 0.0;
        let name = s.topic_name.clone();
        self.save_session();
        if !early { (self.fx.alert)("Time's up", &format!("Rate how well you know: {name}")); }
    }

    fn start_break(&mut self) {
        let now = self.now();
        let p = self.settings.pomo.clone();
        let Some(s) = self.session.as_mut() else { return };
        let long = s.cycles > 0 && s.cycles % (p.cycles.max(1.0) as i64) == 0;
        s.break_kind = Some(if long { "long" } else { "short" }.into());
        s.phase = "break".into();
        s.duration_ms = ((if long { p.long } else { p.brk }) * 60000.0).round();
        s.end_at = now + s.duration_ms;
        s.paused_at = None;
        self.save_session();
    }

    fn complete_break(&mut self) {
        let now = self.now();
        let auto = self.settings.pomo.auto_start_next;
        let Some(s) = self.session.as_mut() else { return };
        s.paused_at = None;
        let continuing = s.topic_id.is_some() && s.blocks.iter().any(|b| b.state == "current") && s.block_index + 1 < s.block_plan.len();
        if continuing {
            s.block_index += 1;
            s.phase = "work".into();
            s.break_kind = None;
            s.duration_ms = (s.block_plan[s.block_index] * 60000.0).round();
            s.end_at = now + s.duration_ms;
            let name = s.topic_name.clone();
            self.save_session();
            (self.fx.alert)("Break over", &format!("Back to {name}"));
            return;
        }
        s.phase = "next".into();
        s.break_kind = None;
        s.end_at = 0.0;
        let next = s.proposal.clone();
        self.save_session();
        (self.fx.alert)("Break over", &match &next { Some(n) => format!("Next: {}", n.name), None => "Pick the next topic".into() });
        if let (Some(n), true) = (next, auto) { self.begin_topic(&n.topic_id); }
    }

    pub fn session_pause(&mut self) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        if (s.phase != "work" && s.phase != "break") || s.paused_at.is_some() { return }
        s.paused_at = Some(now);
        self.save_session();
    }

    pub fn session_resume(&mut self) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let Some(p) = s.paused_at else { return };
        s.end_at += now - p;
        s.paused_at = None;
        self.save_session();
    }

    pub fn session_extend(&mut self, minutes: f64) {
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        if !minutes.is_finite() || minutes <= 0.0 { return }
        if s.phase == "rate" {
            // reopen the block with extra time
            s.phase = "work".into();
            s.duration_ms = (minutes * 60000.0).round();
            s.end_at = now + s.duration_ms;
            self.save_session();
            return;
        }
        if s.phase != "work" && s.phase != "break" { return }
        s.end_at += minutes * 60000.0;
        s.duration_ms += minutes * 60000.0;
        self.save_session();
    }

    pub fn session_end_early(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.phase == "work") { self.complete_work(true) }
    }

    /// Skip the current topic: records time spent only. During a break it skips the break.
    pub fn session_skip(&mut self) {
        let Some(s) = &self.session else { return };
        match s.phase.as_str() {
            "break" => self.complete_break(),
            "next" => { if let Some(p) = s.proposal.clone() { self.session_accept(Some(p.topic_id)) } }
            "work" | "rate" => self.session_rate(None, None, None),
            _ => {}
        }
    }

    /// Pass on the suggested next topic and propose the next-ranked one instead.
    pub fn session_skip_suggestion(&mut self) {
        let Some(s) = &self.session else { return };
        let Some(p) = &s.proposal else { return };
        if s.phase != "next" && s.phase != "break" { return }
        let mut passed = s.passed.clone();
        passed.push(p.topic_id.clone());
        let next = self.suggest(&s.pool, s.sandbox, usize::MAX).into_iter().find(|x| !passed.contains(&x.topic_id));
        let s = self.session.as_mut().expect("session");
        match next {
            None => s.message = Some("No other topics to suggest in this pool.".into()),
            Some(n) => { s.passed = passed; s.proposal = Some(n); s.message = None; }
        }
        self.save_session();
    }

    pub fn session_skip_break(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.phase == "break") { self.complete_break() }
    }

    pub fn session_rate(&mut self, rating: Option<Rating>, quiz: Option<f64>, note: Option<String>) {
        let elapsed = self.elapsed_ms();
        let at = self.now();
        let Some(s) = self.session.as_mut() else { return };
        let Some(topic_id) = s.topic_id.clone() else { return };
        if s.phase != "rate" && s.phase != "work" { return }
        if s.phase == "work" { s.work_ms += elapsed; }
        let minutes = round1(s.work_ms / 60000.0);
        let sandbox = s.sandbox;
        let t = self.store.get::<Topic>("topics", &topic_id);
        if let Some(t) = &t {
            if sandbox {
                self.session.as_mut().expect("session").sandbox_reviews.push(PendingReview { topic_id: t.id.clone(), rating, quiz, minutes, note: note.clone(), at });
            } else {
                self.review(&t.id, &ReviewInput { rating, quiz, minutes, note: note.clone(), at }, true);
            }
        }
        let s = self.session.as_mut().expect("session");
        if let Some(cur) = s.blocks.iter_mut().find(|b| b.state == "current") {
            cur.state = if rating.is_some() { "rated" } else { "skipped" }.into();
            cur.rating = rating;
            cur.minutes = minutes;
        }
        s.last_topic_id = Some(topic_id);
        s.last_doc_id = t.as_ref().map(|t| t.doc_id.clone());
        s.last_was_review = t.as_ref().is_some_and(|t| t.due.is_some_and(|d| d != 0.0));
        s.work_ms = 0.0;
        s.paused_at = None;
        let (pool, sb) = (s.pool.clone(), s.sandbox);
        let proposal = self.suggest(&pool, sb, 1).into_iter().next();
        let s = self.session.as_mut().expect("session");
        s.proposal = proposal;
        if s.mode == "pomodoro" {
            if s.block_index + 1 < s.block_plan.len() { s.cycles += 1; } // skipped mid-topic: count the partial block
            self.start_break();
            return;
        }
        s.phase = "next".into();
        s.end_at = 0.0;
        self.save_session();
    }

    pub fn session_accept(&mut self, topic_id: Option<String>) {
        let Some(s) = self.session.as_mut() else { return };
        let Some(id) = topic_id.or_else(|| s.proposal.as_ref().map(|p| p.topic_id.clone())) else {
            s.message = Some("Nothing left to suggest in this pool.".into());
            self.save_session();
            return;
        };
        if s.phase == "work" { return }
        self.begin_topic(&id);
    }

    /// End the session. Running work time is recorded (time only). Sandbox sessions wait for save/discard.
    pub fn session_end(&mut self) {
        let elapsed = self.elapsed_ms();
        let now = self.now();
        let Some(s) = self.session.as_mut() else { return };
        if s.phase == "work" || s.phase == "rate" {
            if s.phase == "work" { s.work_ms += elapsed; }
            if let (true, Some(tid)) = (s.work_ms > 30000.0, s.topic_id.clone()) {
                let minutes = round1(s.work_ms / 60000.0);
                if let Some(cur) = s.blocks.iter_mut().find(|b| b.state == "current") { cur.state = "skipped".into(); cur.minutes = minutes; }
                if s.sandbox {
                    s.sandbox_reviews.push(PendingReview { topic_id: tid, rating: None, quiz: None, minutes, note: None, at: now });
                } else {
                    self.review(&tid, &ReviewInput { rating: None, quiz: None, minutes, note: None, at: now }, true);
                }
            }
        }
        let s = self.session.as_mut().expect("session");
        if s.sandbox && !s.sandbox_reviews.is_empty() {
            s.phase = "done".into();
            s.end_at = 0.0;
            s.paused_at = None;
            s.blocks.retain(|b| b.state != "current");
            self.save_session();
            return;
        }
        self.session = None;
        self.save_session();
    }

    pub fn session_sandbox(&mut self, save: bool) {
        let Some(s) = self.session.take() else { return };
        if save { for r in &s.sandbox_reviews { self.review(&r.topic_id, &review_of(r), true); } }
        self.save_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Fx;
    use crate::store::Store;

    fn core_with_topics(n: usize) -> Core {
        let c = Core::new(Store::memory(), Fx::default());
        for i in 0..n {
            let mut t = c.new_topic("d1", "c1", &format!("Topic {i}"));
            t.ord = i as f64;
            c.save_topic(&t, Some("some text here"));
        }
        c
    }

    #[test]
    fn timed_session_rates_and_moves_on() {
        let mut c = core_with_topics(3);
        let s = c.session_start("timed", Pool::Global, "All".into(), None).unwrap();
        assert_eq!(s.phase, "work");
        let first = s.topic_id.clone().unwrap();
        c.session_end_early();
        assert_eq!(c.session.as_ref().unwrap().phase, "rate");
        c.session_rate(Some(Rating::Okay), None, None);
        let s = c.session.clone().unwrap();
        assert_eq!(s.phase, "next");
        assert!(s.proposal.as_ref().is_some_and(|p| p.topic_id != first));
        assert_eq!(c.store.get::<Topic>("topics", &first).unwrap().reviews.len(), 1);
        c.session_accept(None);
        assert_eq!(c.session.as_ref().unwrap().phase, "work");
        c.session_end();
        assert!(c.session.is_none());
    }

    #[test]
    fn sandbox_keeps_reviews_until_saved() {
        let mut c = core_with_topics(2);
        let ids: Vec<String> = c.store.all::<Topic>("topics").into_iter().map(|t| t.id).collect();
        c.session_start("timed", Pool::Sandbox { ids: ids.clone() }, "Try".into(), None).unwrap();
        c.session_rate(Some(Rating::Solid), None, None);
        assert!(c.store.all::<Topic>("topics").iter().all(|t| t.reviews.is_empty()));
        c.session_end();
        assert_eq!(c.session.as_ref().unwrap().phase, "done");
        c.session_sandbox(true);
        assert_eq!(c.store.all::<Topic>("topics").iter().filter(|t| !t.reviews.is_empty()).count(), 1);
    }

    #[test]
    fn pomodoro_plans() {
        let mut p = PomoSettings { work: 25.0, brk: 5.0, long: 15.0, cycles: 4.0, per_topic: "minutes".into(), n: 3.0, minutes: 40.0, auto_start_next: false };
        assert_eq!(pomodoro_plan(&p), vec![25.0, 15.0]);
        p.per_topic = "n".into();
        assert_eq!(pomodoro_plan(&p).len(), 3);
    }
}
