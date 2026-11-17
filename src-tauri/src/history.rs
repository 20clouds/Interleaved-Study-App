// History carry-over: match new topics to old ones (same name, or text Jaccard on word shingles >= 0.6)
// and copy their study history so re-imports and re-organization never silently lose progress.
use crate::types::Topic;
use crate::util::norm_name;
use std::collections::{HashMap, HashSet};

pub const MATCH_THRESHOLD: f64 = 0.6;

pub fn shingles(text: &str) -> HashSet<String> {
    let norm = norm_name(text);
    let w: Vec<&str> = norm.split(' ').filter(|x| !x.is_empty()).collect();
    let mut out = HashSet::new();
    if w.is_empty() { return out }
    if w.len() < 3 { out.insert(w.join(" ")); return out }
    for i in 0..=w.len() - 3 { out.insert(w[i..i + 3].join(" ")); }
    out
}

pub fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() { return 0.0 }
    let (small, big) = if a.len() < b.len() { (a, b) } else { (b, a) };
    let inter = small.iter().filter(|x| big.contains(*x)).count();
    inter as f64 / (a.len() + b.len() - inter) as f64
}

pub struct MatchItem {
    pub id: String,
    pub name: String,
    pub text: String,
}

/// Returns newId -> oldId. Exact (normalized) name matches first, then best text similarity >= threshold, greedily.
pub fn match_topics(olds: &[MatchItem], news: &[MatchItem]) -> HashMap<String, String> {
    let mut result: HashMap<String, String> = HashMap::new();
    let mut used_old: HashSet<String> = HashSet::new();
    let mut by_name: HashMap<String, Vec<&MatchItem>> = HashMap::new();
    for o in olds { by_name.entry(norm_name(&o.name)).or_default().push(o); }
    for n in news {
        if let Some(c) = by_name.get(&norm_name(&n.name)).and_then(|l| l.iter().find(|o| !used_old.contains(&o.id))) {
            result.insert(n.id.clone(), c.id.clone());
            used_old.insert(c.id.clone());
        }
    }
    let old_sh: Vec<(&MatchItem, HashSet<String>)> = olds.iter().map(|o| (o, shingles(&o.text))).collect();
    let mut pairs: Vec<(String, String, f64)> = Vec::new();
    for n in news {
        if result.contains_key(&n.id) { continue }
        let ns = shingles(&n.text);
        for (o, sh) in &old_sh {
            if used_old.contains(&o.id) { continue }
            let s = jaccard(&ns, sh);
            if s >= MATCH_THRESHOLD { pairs.push((n.id.clone(), o.id.clone(), s)); }
        }
    }
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    for (n, o, _) in pairs {
        if result.contains_key(&n) || used_old.contains(&o) { continue }
        used_old.insert(o.clone());
        result.insert(n, o);
    }
    result
}

pub fn has_history(t: &Topic) -> bool {
    !t.reviews.is_empty() || t.spent > 0.0 || t.due.is_some_and(|d| d != 0.0) || !t.notes.trim().is_empty() || t.override_score.is_some()
}

pub fn carry_history(target: &Topic, from: &Topic) -> Topic {
    let mut out = target.clone();
    out.stab = from.stab;
    out.due = from.due;
    out.last = from.last;
    out.evid = from.evid;
    out.reviews = from.reviews.clone();
    out.spent = from.spent;
    out.override_score = from.override_score;
    out.notes = from.notes.clone();
    out.solid_mark = from.solid_mark;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, name: &str, text: &str) -> MatchItem {
        MatchItem { id: id.into(), name: name.into(), text: text.into() }
    }

    #[test]
    fn matches_by_name_then_text() {
        let text = "the quick brown fox jumps over the lazy dog near the river bank today";
        let olds = vec![item("o1", "Foxes", text), item("o2", "Glycolysis", "glucose is split into pyruvate in ten steps")];
        let news = vec![item("n1", "glycolysis!", "something else entirely"), item("n2", "Renamed", text), item("n3", "Other", "no match at all here")];
        let m = match_topics(&olds, &news);
        assert_eq!(m.get("n1").map(String::as_str), Some("o2"));
        assert_eq!(m.get("n2").map(String::as_str), Some("o1"));
        assert!(!m.contains_key("n3"));
    }
}
