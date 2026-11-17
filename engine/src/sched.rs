// FSRS-lite scheduler and mastery model. Pure functions; mastery and status are derived on read.
use crate::types::{Rating, Review, Suggestion, Topic};
use crate::util::{jround, round1, DAY};

pub fn evidence(r: Rating) -> f64 {
    match r { Rating::Lost => 0.1, Rating::Shaky => 0.4, Rating::Okay => 0.7, Rating::Solid => 1.0 }
}

pub fn categorize(e: f64) -> Rating {
    if e < 0.25 { Rating::Lost } else if e < 0.55 { Rating::Shaky } else if e < 0.85 { Rating::Okay } else { Rating::Solid }
}

pub fn combined_evidence(own: Rating, quiz: Option<f64>) -> f64 {
    let s = evidence(own);
    match quiz {
        Some(q) if !q.is_nan() => (s + 2.0 * q) / 3.0,
        _ => s,
    }
}

pub fn next_stability(rating: Rating, s0: Option<f64>) -> f64 {
    let s = match s0 { Some(x) if x > 0.0 => x, _ => 0.5 };
    match rating {
        Rating::Lost => (0.3 * s).max(0.25),
        Rating::Shaky => (1.2 * s).max(0.5),
        Rating::Okay => (2.2 * s).max(1.0),
        Rating::Solid => (3.5 * s).max(3.0),
    }
}

#[derive(Clone, Debug)]
pub struct ReviewInput {
    pub rating: Option<Rating>, // None = skip rating, record time only
    pub quiz: Option<f64>,
    pub minutes: f64,
    pub note: Option<String>,
    pub at: f64,
}

/// Apply a review. A skipped rating only records time (no history entry is added).
pub fn apply_review(t: &Topic, r: &ReviewInput) -> Topic {
    let mut n = t.clone();
    n.spent = t.spent + r.minutes.max(0.0);
    let Some(rating) = r.rating else { return n };
    let ev = combined_evidence(rating, r.quiz);
    let cat = categorize(ev);
    let stab = next_stability(cat, t.stab);
    n.stab = Some(stab);
    n.due = Some(r.at + stab * DAY);
    n.last = Some(r.at);
    n.evid = Some(ev);
    n.reviews.push(Review {
        at: r.at, rating: cat, self_rating: Some(rating), minutes: round1(r.minutes), quiz: r.quiz,
        note: r.note.clone().filter(|x| !x.is_empty()),
    });
    n.override_score = None;
    n.solid_mark = Some(false);
    n
}

fn has(v: Option<f64>) -> bool {
    matches!(v, Some(x) if x != 0.0)
}

pub fn recall(t: &Topic, now: f64) -> f64 {
    if !has(t.last) || !has(t.stab) { return 0.0 }
    let days = ((now - t.last.unwrap()) / DAY).max(0.0);
    0.9f64.powf(days / t.stab.unwrap())
}

pub fn mastery(t: &Topic, now: f64) -> f64 {
    if let Some(o) = t.override_score { return jround(o) }
    if !has(t.last) || !has(t.stab) { return 0.0 }
    let r = recall(t, now);
    let c = if t.solid_mark == Some(true) { 1.0 } else { 0.6 + 0.4 * (t.reviews.len() as f64 / 3.0).min(1.0) };
    jround(100.0 * r * t.evid.unwrap_or(0.0) * c)
}

/// new | progress | review | solid
pub fn status(t: &Topic, now: f64) -> &'static str {
    if !has(t.due) { return "new" }
    if mastery(t, now) >= 80.0 { return "solid" }
    if t.due.unwrap() <= now { "review" } else { "progress" }
}

pub fn last_rating(t: &Topic) -> Option<Rating> {
    t.reviews.last().map(|r| r.rating)
}

pub fn set_due(t: &Topic, at: f64, now: f64) -> Topic {
    let mut n = t.clone();
    n.due = Some(at);
    n.stab = Some(t.stab.unwrap_or(((at - now) / DAY).max(0.25)));
    n.last = Some(t.last.unwrap_or(now));
    n.evid = Some(t.evid.unwrap_or(0.4));
    n
}

pub fn mark_solid(t: &Topic, now: f64) -> Topic {
    let mut n = t.clone();
    n.stab = Some(30.0);
    n.evid = Some(1.0);
    n.last = Some(now);
    n.due = Some(now + 30.0 * DAY);
    n.solid_mark = Some(true);
    n.override_score = None;
    n
}

pub fn reset_sched(t: &Topic) -> Topic {
    let mut n = t.clone();
    n.stab = None;
    n.due = None;
    n.last = None;
    n.evid = None;
    n.reviews = vec![];
    n.spent = 0.0;
    n.override_score = None;
    n.solid_mark = Some(false);
    n
}

#[derive(Clone, Debug)]
pub struct PickDeadline {
    pub title: String,
    pub date: f64, // ms timestamp of the deadline day
    pub course_id: Option<String>,
    pub doc_id: Option<String>,
    pub topic_ids: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct PickCtx {
    pub now: f64,
    pub last_topic_id: Option<String>,
    pub last_doc_id: Option<String>,
    pub last_was_review: bool,
    pub deadlines: Vec<PickDeadline>,
}

pub const DEADLINE_HORIZON_DAYS: f64 = 21.0;

/// Priority added for a topic linked to a deadline: grows linearly from 0 (21 days out) to 1.2 (due today).
pub fn deadline_pressure<'a>(t: &Topic, deadlines: &'a [PickDeadline], now: f64) -> (f64, Option<&'a PickDeadline>, f64) {
    let mut best: (f64, Option<&PickDeadline>, f64) = (0.0, None, 0.0);
    for d in deadlines {
        let linked = d.topic_ids.iter().any(|x| x == &t.id)
            || d.doc_id.as_deref().is_some_and(|x| !x.is_empty() && x == t.doc_id)
            || d.course_id.as_deref().is_some_and(|x| !x.is_empty() && x == t.course_id);
        if !linked { continue }
        let days = (d.date - now) / DAY;
        if !(-1.0..=DEADLINE_HORIZON_DAYS).contains(&days) { continue }
        let p = 1.2 * (1.0 - days.max(0.0) / DEADLINE_HORIZON_DAYS).clamp(0.0, 1.0);
        if p > best.0 { best = (p, Some(d), days) }
    }
    best
}

pub fn score_topic(t: &Topic, ctx: &PickCtx) -> Suggestion {
    let now = ctx.now;
    let mut score: f64;
    let mut reason: String;
    if !has(t.due) {
        score = 0.8 + if ctx.last_was_review { 0.3 } else { 0.0 };
        reason = if ctx.last_was_review { "New material, mixed in after a review.".into() } else { "New material.".into() };
    } else if t.due.unwrap() <= now {
        let due = t.due.unwrap();
        let stab_ms = t.stab.unwrap_or(0.5).max(0.25) * DAY;
        let ratio = (now - due) / stab_ms;
        score = 1.0 + ratio.min(3.0);
        let lr = last_rating(t).map(|r| format!("; last rated {}", r.label())).unwrap_or_default();
        reason = format!("Overdue by {:.1} days{lr}.", (now - due) / DAY);
    } else {
        let r = recall(t, now);
        score = (1.0 - r) * 0.5;
        reason = format!("Not due yet (in {:.1} days); estimated recall {}%.", (t.due.unwrap() - now) / DAY, jround(r * 100.0));
    }
    if !ctx.deadlines.is_empty() {
        let (p, d, days) = deadline_pressure(t, &ctx.deadlines, now);
        if let (true, Some(d)) = (p > 0.0, d) {
            score += p;
            let when = if days <= 0.0 { "is today".to_string() } else { format!("in {} days", days.ceil()) };
            reason += &format!(" Deadline \"{}\" {when}.", d.title);
        }
    }
    if ctx.last_doc_id.as_deref().is_some_and(|x| x == t.doc_id) && ctx.last_topic_id.as_deref() != Some(t.id.as_str()) {
        score -= 0.3;
        reason += " Same lesson as the last block, so slightly lower.";
    }
    if ctx.last_topic_id.as_deref() == Some(t.id.as_str()) {
        score -= 9.0;
        reason = "Just studied; avoided back to back.".into();
    }
    Suggestion { topic_id: t.id.clone(), name: t.name.clone(), score: jround(score * 1000.0) / 1000.0, reason }
}

pub fn rank(topics: &[Topic], ctx: &PickCtx) -> Vec<Suggestion> {
    let mut scored: Vec<(Suggestion, f64)> = topics.iter().map(|t| (score_topic(t, ctx), t.ord)).collect();
    scored.sort_by(|a, b| b.0.score.partial_cmp(&a.0.score).unwrap_or(std::cmp::Ordering::Equal).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
    scored.into_iter().map(|x| x.0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topic(id: &str) -> Topic {
        Topic { id: id.into(), doc_id: "d".into(), course_id: "c".into(), name: id.into(), ..Default::default() }
    }

    #[test]
    fn review_schedules_and_records() {
        let t = topic("a");
        let at = 1_000_000.0;
        let n = apply_review(&t, &ReviewInput { rating: Some(Rating::Okay), quiz: None, minutes: 12.34, note: None, at });
        assert_eq!(n.stab, Some(1.1));
        assert_eq!(n.due, Some(at + 1.1 * DAY));
        assert_eq!(n.reviews.len(), 1);
        assert_eq!(n.reviews[0].minutes, 12.3);
        assert_eq!(status(&n, at), "progress");
        assert_eq!(status(&n, at + 2.0 * DAY), "review");
        // a skipped rating only adds time
        let s = apply_review(&n, &ReviewInput { rating: None, quiz: None, minutes: 5.0, note: None, at });
        assert_eq!(s.reviews.len(), 1);
        assert!((s.spent - 17.34).abs() < 1e-9);
    }

    #[test]
    fn quiz_counts_double() {
        assert!((combined_evidence(Rating::Solid, Some(0.0)) - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(categorize(combined_evidence(Rating::Solid, Some(0.0))), Rating::Shaky);
    }

    #[test]
    fn ranking_prefers_overdue_and_avoids_repeats() {
        let now = 100.0 * DAY;
        let mut a = topic("a");
        a.due = Some(now - DAY);
        a.stab = Some(1.0);
        a.last = Some(now - 2.0 * DAY);
        let b = topic("b");
        let r = rank(&[b.clone(), a.clone()], &PickCtx { now, ..Default::default() });
        assert_eq!(r[0].topic_id, "a");
        let r = rank(&[b, a], &PickCtx { now, last_topic_id: Some("a".into()), ..Default::default() });
        assert_eq!(r[0].topic_id, "b");
    }

    #[test]
    fn solid_mark_gives_full_mastery() {
        let now = 5.0 * DAY;
        let n = mark_solid(&topic("a"), now);
        assert_eq!(mastery(&n, now), 100.0);
        assert_eq!(status(&n, now), "solid");
    }
}
