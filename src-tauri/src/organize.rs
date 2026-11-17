// AI-organized import: prompts, validation/repair of paragraph ranges, merging of continuing topics,
// and local de-duplication. The model never re-outputs source text; we slice locally, at paragraph edges for the
// topic text and at the opening words the model quotes for each topic's exact excerpt.
use crate::aijson::{cap, snippet, str_list, str_of, InvalidJson};
use crate::preprocess::{first_sentence_name, safe_name, slice_text};
use crate::rx;
use crate::types::Para;
use crate::util::{clamp, is_generic_name, jround, norm_name, sha256};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const PROMPT_VERSION: &str = "org-v4";

pub const ORGANIZE_SYSTEM: &str = "You organize study material into clear study topics.
Input: numbered paragraphs labeled [pN] plus the document outline.
Return ONLY compact JSON, no prose, no code fences:
{\"title\":string?,\"topics\":[{\"start\":N,\"end\":N,\"from\":string,\"name\":string,\"unit\":string,\"summary\":string,\"difficulty\":1-5,\"terms\":[string],\"prereqs\":[string],\"continues\":bool}]}
Rules: topics cover every paragraph of the window in order, no gaps or overlaps; start/end are paragraph numbers.
from: the first 6-10 words of the topic, copied exactly from the text where the topic begins. A topic may begin partway through a paragraph: give that paragraph to whichever topic has most of it, and still quote the exact words where this topic begins.
name: a specific concept title of at most 8 words (e.g. \"Michaelis-Menten enzyme kinetics\"); never generic like \"Part 2\", \"Section 3\", \"Introduction continued\".
unit: the section/chapter the topic belongs to. summary: 1-2 sentences. terms: up to 6 key terms. prereqs: up to 3 prerequisite concepts.
Aim for topics of roughly 300-1500 words. When headings are good, treat them as strong boundaries and mainly clean names and group units; split or merge only when clearly warranted.
continues: true only for the first topic if it continues the previous window's last topic.
Do not repeat the source text.";

fn difficulty() -> Value { json!({ "type": "integer", "minimum": 1, "maximum": 5 }) }
fn terms() -> Value { json!({ "type": "array", "items": { "type": "string" }, "maxItems": 6 }) }
fn prereqs() -> Value { json!({ "type": "array", "items": { "type": "string" }, "maxItems": 3 }) }

/// Gemini response schemas mirror each prompt's JSON shape in the same key order; all fields are required.
pub fn organize_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": ["string", "null"] },
            "topics": { "type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {
                    "start": { "type": "integer" }, "end": { "type": "integer" }, "from": { "type": "string" }, "name": { "type": "string" },
                    "unit": { "type": "string" }, "summary": { "type": "string" }, "difficulty": difficulty(), "terms": terms(),
                    "prereqs": prereqs(), "continues": { "type": "boolean" }
                },
                "required": ["start", "end", "from", "name", "unit", "summary", "difficulty", "terms", "prereqs", "continues"]
            } }
        },
        "required": ["title", "topics"]
    })
}

pub struct OrganizePrompt<'a> {
    pub lesson_title: &'a str,
    pub outline: &'a str,
    pub paras: &'a [Para],
    pub from: i64,
    pub to: i64,
    pub prev_topic: Option<&'a str>,
    pub first: bool,
}

pub fn organize_prompt(o: &OrganizePrompt) -> String {
    let lines: Vec<String> = o.paras.iter().filter(|p| p.id >= o.from && p.id <= o.to).map(|p| format!("[p{}] {}", p.id, p.text)).collect();
    let mut parts: Vec<String> = vec![format!("Lesson file: {}", o.lesson_title)];
    if o.first { parts.push("Also propose \"title\": a clean lesson title (max 8 words).".into()); }
    parts.push(if o.outline.is_empty() { "Outline: (no headings detected)".into() } else { format!("Outline:\n{}", o.outline) });
    parts.push(match (o.prev_topic, o.first) {
        (Some(p), _) => format!("Previous window ended with topic: \"{p}\""),
        (None, true) => "This is the first window.".into(),
        // Windows answered together (a scheduled-task batch) do not know each other's topics yet.
        (None, false) => "This is a later window; the previous window's last topic is not known. Set continues to true when the first paragraphs clearly carry on a topic from before.".into(),
    });
    parts.push(format!("Paragraphs p{}-p{}:", o.from, o.to));
    parts.push(lines.join("\n"));
    parts.join("\n\n")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WinTopic {
    pub start: i64,
    pub end: i64,
    #[serde(default)]
    pub from: String, // the topic's opening words as quoted by the model ('' when missing)
    pub name: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub difficulty: f64,
    #[serde(default)]
    pub terms: Vec<String>,
    #[serde(default)]
    pub prereqs: Vec<String>,
    #[serde(default)]
    pub continues: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WinResult {
    #[serde(default)]
    pub title: Option<String>,
    pub topics: Vec<WinTopic>,
}

fn to_int(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_f64().filter(|x| x.is_finite()).map(|x| jround(x) as i64),
        Some(Value::String(s)) => rx!(r"\d+").find(s).and_then(|m| m.as_str().parse::<i64>().ok()),
        _ => None,
    }
}

fn cap_words(s: &str, n: usize) -> String {
    s.split_whitespace().take(n).collect::<Vec<_>>().join(" ")
}

fn union(a: &[String], b: &[String], max: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    a.iter().chain(b.iter()).filter(|x| seen.insert((*x).clone())).take(max).cloned().collect()
}

fn pick<'a>(r: &'a Value, a: &str, b: &str) -> Option<&'a Value> {
    match r.get(a) { Some(Value::Null) | None => r.get(b), v => v }
}

/// Validate a model's window output against the window range and repair it locally:
/// clamp to range, sort, remove overlaps, attach gaps to the nearest topic, fix generic names.
pub fn normalize_window(raw: &Value, from: i64, to: i64, paras: &[Para], lesson_title: &str) -> Result<WinResult, InvalidJson> {
    let wrapped;
    let obj = if raw.is_array() { wrapped = json!({ "topics": raw }); &wrapped } else { raw };
    let Some(topics) = obj.get("topics").and_then(|x| x.as_array()) else {
        return Err(InvalidJson(format!("Expected {{\"topics\":[...]}} ({})", snippet(raw, 300))));
    };
    let mut list: Vec<WinTopic> = Vec::new();
    for r in topics {
        if !r.is_object() { continue }
        let (Some(mut start), Some(mut end)) = (to_int(pick(r, "start", "s")), to_int(pick(r, "end", "e"))) else { continue };
        if start > end { std::mem::swap(&mut start, &mut end); }
        let name = str_of(r.get("name"), cap::NAME);
        list.push(WinTopic {
            start: start.clamp(from, to),
            end: end.clamp(from, to),
            from: str_of(r.get("from"), cap::QUOTE),
            name: cap_words(&name, 10),
            unit: str_of(r.get("unit"), cap::NAME),
            summary: str_of(r.get("summary"), cap::SUMMARY),
            difficulty: clamp(to_int(pick(r, "difficulty", "d")).unwrap_or(3) as f64, 1.0, 5.0),
            terms: str_list(r.get("terms"), 6, cap::TERM),
            prereqs: str_list(pick(r, "prereqs", "pre"), 3, cap::TERM),
            continues: r.get("continues") == Some(&Value::Bool(true)) || r.get("continues").and_then(|x| x.as_str()) == Some("true"),
        });
    }
    if list.is_empty() { return Err(InvalidJson(format!("No valid topics with paragraph ranges ({})", snippet(raw, 300)))) }
    list.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    // Remove overlaps
    let mut fixed: Vec<WinTopic> = Vec::new();
    for mut t in list {
        if let Some(prev) = fixed.last_mut() {
            if t.start <= prev.end {
                if t.end <= prev.end { // fully contained: drop, keep terms
                    prev.terms = union(&prev.terms, &t.terms, 6);
                    continue;
                }
                t.start = prev.end + 1;
            }
        }
        fixed.push(t);
    }
    // Fill gaps: leading gap attaches to the first topic; inner/trailing gaps attach to the previous topic
    fixed[0].start = from;
    for i in 1..fixed.len() {
        if fixed[i].start > fixed[i - 1].end + 1 { fixed[i - 1].end = fixed[i].start - 1; }
    }
    let last = fixed.len() - 1;
    fixed[last].end = to;
    for t in fixed.iter_mut() {
        let body = slice_text(paras, t.start, t.end);
        t.name = safe_name(&t.name, &body, lesson_title);
        if t.unit.is_empty() || is_generic_name(&t.unit) { t.unit = String::new(); }
    }
    let title = str_of(obj.get("title"), cap::NAME);
    Ok(WinResult { title: if !title.is_empty() && !is_generic_name(&title) { Some(cap_words(&title, 10)) } else { None }, topics: fixed })
}

#[derive(Clone, Debug)]
pub struct FinalTopic {
    pub t: WinTopic,
    pub words: i64,
}

fn merge_into(a: &mut WinTopic, b: &WinTopic) {
    a.end = a.end.max(b.end);
    a.start = a.start.min(b.start);
    a.terms = union(&a.terms, &b.terms, 6);
    a.prereqs = union(&a.prereqs, &b.prereqs, 3);
    if !b.summary.is_empty() && !a.summary.is_empty() && !a.summary.contains(&b.summary) && a.summary.chars().count() < 250 {
        a.summary = format!("{} {}", a.summary, b.summary);
    }
    if a.summary.is_empty() { a.summary = b.summary.clone(); }
    a.difficulty = jround((a.difficulty + b.difficulty) / 2.0);
}

/// Combine window results into lesson topics: merge continuing topics, same-name neighbours and tiny topics.
pub fn finalize_windows(results: &[WinResult], paras: &[Para], lesson_title: &str) -> Vec<FinalTopic> {
    let min_words = 60;
    let mut all: Vec<WinTopic> = Vec::new();
    for (wi, r) in results.iter().enumerate() {
        for (ti, t) in r.topics.iter().enumerate() {
            if let Some(prev) = all.last_mut() {
                if wi > 0 && ti == 0 && (t.continues || norm_name(&t.name) == norm_name(&prev.name)) {
                    merge_into(prev, t);
                    continue;
                }
            }
            all.push(t.clone());
        }
    }
    // Local de-duplication of adjacent identical names
    let mut dedup: Vec<WinTopic> = Vec::new();
    for t in all {
        if let Some(prev) = dedup.last_mut() {
            if norm_name(&prev.name) == norm_name(&t.name) { merge_into(prev, &t); continue }
        }
        dedup.push(t);
    }
    // Merge tiny topics into a neighbour
    let words = |t: &WinTopic| paras.iter().filter(|p| p.id >= t.start && p.id <= t.end && p.heading.is_none()).map(|p| p.words).sum::<i64>();
    let mut out: Vec<WinTopic> = Vec::new();
    let n = dedup.len();
    let mut i = 0;
    while i < n {
        let t = dedup[i].clone();
        if words(&t) < min_words && n > 1 {
            if let Some(prev) = out.last_mut() { merge_into(prev, &t); i += 1; continue }
            if i + 1 < n {
                dedup[i + 1].start = t.start;
                dedup[i + 1].from = t.from.clone();
                i += 1;
                continue;
            }
        }
        out.push(t);
        i += 1;
    }
    // Units: fill missing units from the previous topic, else lesson title
    let mut unit = lesson_title.to_string();
    for t in out.iter_mut() {
        if !t.unit.is_empty() { unit = t.unit.clone(); } else { t.unit = unit.clone(); }
    }
    // Disambiguate repeated names within the lesson
    let mut seen: HashMap<String, usize> = HashMap::new();
    for t in out.iter_mut() {
        let k = norm_name(&t.name);
        let c = seen.entry(k.clone()).or_insert(0);
        *c += 1;
        if *c > 1 {
            let body = slice_text(paras, t.start, t.end);
            let rest = match body.find('\n') { Some(p) => &body[p + 1..], None => body.as_str() };
            let alt = first_sentence_name(rest);
            t.name = if !alt.is_empty() && norm_name(&alt) != k { alt } else { format!("{} ({})", t.name, t.unit) };
        }
    }
    out.into_iter().map(|t| {
        let w = words(&t);
        let w = if w != 0 { w } else { paras.iter().filter(|p| p.id >= t.start && p.id <= t.end).map(|p| p.words).sum() };
        FinalTopic { t, words: w }
    }).collect()
}

/// Lower-cased letters and digits, every other run of characters as one space, and where each character came from.
struct Folded {
    norm: Vec<char>,
    at: Vec<usize>,
}

fn fold(s: &[char]) -> Folded {
    let mut norm = Vec::with_capacity(s.len());
    let mut at = Vec::with_capacity(s.len());
    let mut space = true;
    for (i, ch) in s.iter().enumerate() {
        if ch.is_alphanumeric() {
            for c in ch.to_lowercase() { norm.push(c); at.push(i); }
            space = false;
        } else if !space {
            norm.push(' ');
            at.push(i);
            space = true;
        }
    }
    Folded { norm, at }
}

fn find_from(hay: &[char], needle: &[char], start: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() { return None }
    (start..=hay.len() - needle.len()).find(|&k| hay[k] == needle[0] && hay[k..k + needle.len()] == *needle)
}

/// Where `quote` starts in the text between offsets lo and hi, nearest to `near`, or None. Matching ignores case,
/// punctuation and spacing, and starts on a word. If the whole quote is not there (the model tidied a word), its
/// first 6, then 4 words are tried.
fn find_quote(f: &Folded, quote: &str, lo: usize, hi: usize, near: usize) -> Option<usize> {
    let qchars: Vec<char> = quote.chars().collect();
    let folded: String = fold(&qchars).norm.iter().collect();
    let words: Vec<&str> = folded.trim().split(' ').filter(|w| !w.is_empty()).collect();
    for n in [words.len(), 6, 4] {
        if n > words.len() || n < 3 { continue }
        let q: Vec<char> = words[..n].join(" ").chars().collect();
        let mut best: Option<usize> = None;
        let mut k = find_from(&f.norm, &q, 0);
        while let Some(pos_n) = k {
            if pos_n == 0 || f.norm[pos_n - 1] == ' ' {
                let pos = f.at[pos_n];
                if pos >= lo && pos < hi && best.map_or(true, |b| pos.abs_diff(near) < b.abs_diff(near)) { best = Some(pos); }
            }
            k = find_from(&f.norm, &q, pos_n + 1);
        }
        if best.is_some() { return best }
    }
    None
}

pub struct ExcerptTopic<'a> {
    pub start: i64,
    pub end: i64,
    pub from: &'a str,
}

/// Each topic's verbatim section of the lesson: cut where the model's quote (`from`) begins instead of at a paragraph
/// edge. A quote may sit in the topic's first paragraph or the one before it; one that cannot be found there falls
/// back to the first paragraph's start. The sections follow one another, so together they cover the lesson once.
pub fn topic_excerpts(paras: &[Para], topics: &[ExcerptTopic]) -> Vec<String> {
    if topics.is_empty() { return vec![] }
    let full: Vec<char> = paras.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n").chars().collect();
    let mut start_of: HashMap<i64, usize> = HashMap::new();
    let mut end_of: HashMap<i64, usize> = HashMap::new();
    let mut off = 0usize;
    for p in paras {
        let n = p.text.chars().count();
        start_of.insert(p.id, off);
        end_of.insert(p.id, off + n);
        off += n + 2;
    }
    let para_start = |id: i64| -> usize {
        start_of.get(&id).copied().or_else(|| paras.iter().find(|p| p.id >= id).and_then(|p| start_of.get(&p.id).copied())).unwrap_or(full.len())
    };
    let para_end = |id: i64| -> usize {
        end_of.get(&id).copied().or_else(|| paras.iter().rev().find(|p| p.id <= id).and_then(|p| end_of.get(&p.id).copied())).unwrap_or(full.len())
    };
    let f = fold(&full);
    // A quote found right after a line's "## " or "- " marker starts at the line instead, keeping the marker.
    let line_start = |pos: usize, lo: usize| -> usize {
        let mut k = pos;
        while k > lo && full[k - 1] != '\n' && !full[k - 1].is_alphanumeric() { k -= 1; }
        if k == 0 || full[k - 1] == '\n' { k } else { pos }
    };
    let mut cuts: Vec<usize> = Vec::with_capacity(topics.len());
    for (i, t) in topics.iter().enumerate() {
        let def = para_start(t.start);
        if i == 0 { cuts.push(0); continue }
        let lo = (cuts[i - 1] + 1).max(para_start(t.start - 1));
        let hi = para_end(t.end);
        let found = if t.from.is_empty() { None } else { find_quote(&f, t.from, lo, hi, def) };
        cuts.push(match found { Some(p) => line_start(p, lo), None => def.max(cuts[i - 1] + 1) });
    }
    topics.iter().enumerate().map(|(i, t)| {
        let a = cuts[i].min(full.len());
        let b = if i + 1 < topics.len() { cuts[i + 1] } else { para_end(t.end).max(cuts[i]) }.min(full.len()).max(a);
        full[a..b].iter().collect::<String>().trim().to_string()
    }).collect()
}

fn token_set(s: &str) -> HashSet<String> {
    norm_name(s).split(' ').filter(|w| w.chars().count() > 2).map(String::from).collect()
}

pub fn name_similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (token_set(a), token_set(b));
    if a.is_empty() || b.is_empty() { return 0.0 }
    let inter = a.iter().filter(|x| b.contains(*x)).count();
    inter as f64 / (a.len() + b.len() - inter) as f64
}

/// Adjacent topics in the same unit whose names are similar but not identical: candidates for an AI merge check.
pub fn ambiguous_pairs(topics: &[(String, String)]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 1..topics.len() {
        if out.len() >= 10 { break }
        let (a, b) = (&topics[i - 1], &topics[i]);
        if a.1 != b.1 { continue }
        let s = name_similarity(&a.0, &b.0);
        if (0.5..1.0).contains(&s) { out.push((i - 1, i)); }
    }
    out
}

pub const DEDUPE_SYSTEM: &str = "You decide whether adjacent study topics cover the same concept. Return ONLY JSON: {\"pairs\":[{\"pair\":N,\"same\":bool,\"name\":string}]} where name is the merged title (max 8 words) when same is true, else \"\".";

pub fn dedupe_schema() -> Value {
    json!({ "type": "object", "properties": { "pairs": { "type": "array", "items": {
        "type": "object", "properties": { "pair": { "type": "integer" }, "same": { "type": "boolean" }, "name": { "type": "string" } },
        "required": ["pair", "same", "name"] } } }, "required": ["pairs"] })
}

pub fn dedupe_prompt(pairs: &[((String, String), (String, String))]) -> String {
    pairs.iter().enumerate().map(|(i, (a, b))| format!("Pair {i}:\nA: {} — {}\nB: {} — {}", a.0, a.1, b.0, b.1)).collect::<Vec<_>>().join("\n\n")
}

pub const QUIZ_SYSTEM: &str = "You write short study quizzes from the given text only.
Return ONLY JSON: {\"questions\":[{\"type\":\"mc\",\"q\":string,\"options\":[4 strings],\"answer\":index 0-3,\"why\":string},{\"type\":\"short\",\"q\":string,\"answer\":string}]}
Write 5 questions: 3 multiple choice and 2 short answer. Keep answers short. Test understanding, not trivia.";

pub fn quiz_schema() -> Value {
    json!({ "type": "object", "properties": { "questions": { "type": "array", "minItems": 5, "maxItems": 5, "items": { "anyOf": [
        { "type": "object", "properties": {
            "type": { "type": "string", "enum": ["mc"] }, "q": { "type": "string" },
            "options": { "type": "array", "items": { "type": "string" }, "minItems": 4, "maxItems": 4 },
            "answer": { "type": "integer", "minimum": 0, "maximum": 3 }, "why": { "type": "string" } },
          "required": ["type", "q", "options", "answer", "why"] },
        { "type": "object", "properties": { "type": { "type": "string", "enum": ["short"] }, "q": { "type": "string" }, "answer": { "type": "string" } },
          "required": ["type", "q", "answer"] }
    ] } } }, "required": ["questions"] })
}

pub const GRADE_SYSTEM: &str = "You grade short answers against an answer key. Be fair about wording. Return ONLY JSON: {\"grades\":[{\"correct\":bool,\"feedback\":string}]} with one entry per item, feedback max 20 words.";

/// The item count is pinned for one request (minItems = maxItems = n).
pub fn grade_schema(n: usize) -> Value {
    let k = n.clamp(1, 8);
    json!({ "type": "object", "properties": { "grades": { "type": "array", "minItems": k, "maxItems": k, "items": {
        "type": "object", "properties": { "correct": { "type": "boolean" }, "feedback": { "type": "string" } }, "required": ["correct", "feedback"] } } },
        "required": ["grades"] })
}

pub const ENRICH_SYSTEM: &str = "You describe one study topic from its text.
Return ONLY JSON: {\"name\":string,\"summary\":string,\"difficulty\":1-5,\"terms\":[string],\"prereqs\":[string]}
name: specific concept title (max 8 words, never generic). summary 1-2 sentences, up to 6 terms, up to 3 prereqs.";

pub fn enrich_schema() -> Value {
    json!({ "type": "object", "properties": { "name": { "type": "string" }, "summary": { "type": "string" }, "difficulty": difficulty(), "terms": terms(), "prereqs": prereqs() },
        "required": ["name", "summary", "difficulty", "terms", "prereqs"] })
}

/// Cache key: content + prompt version (+ model only when the setting asks for it).
pub fn cache_key(kind: &str, content: &str, model: &str, include_model: bool) -> String {
    sha256(&[kind, PROMPT_VERSION, if include_model { model } else { "*" }, &sha256(content)].join("|"))
}

pub fn window_cache_content(outline: &str, paras: &[Para], from: i64, to: i64, first: bool) -> String {
    format!("{}\n{}\n{}", if first { "first" } else { "next" }, outline, slice_text(paras, from, to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preprocess::to_paras;

    fn lesson() -> Vec<Para> {
        let a = "Enzymes are biological catalysts that speed up reactions. ".repeat(14);
        let b = "Competitive inhibitors bind the active site and block the substrate. ".repeat(14);
        to_paras(&format!("# Enzymes\n\n{a}\n\n{b}\n\n## Regulation\n\nAllosteric sites change enzyme shape. {a}"))
    }

    #[test]
    fn window_output_is_repaired() {
        let paras = lesson();
        let raw = json!({ "title": "Enzyme basics", "topics": [
            { "start": 3, "end": 9, "name": "Part 2", "unit": "Section 1", "from": "Competitive inhibitors bind", "difficulty": 9 },
            { "start": "p1", "end": 2, "name": "Enzymes as catalysts", "unit": "Enzymes", "terms": ["catalyst"] },
            { "start": 2, "end": 2, "name": "Contained", "terms": ["x"] }
        ] });
        let w = normalize_window(&raw, 1, 5, &paras, "Lesson").unwrap();
        assert_eq!(w.title.as_deref(), Some("Enzyme basics"));
        assert_eq!(w.topics.len(), 2);
        assert_eq!((w.topics[0].start, w.topics[0].end), (1, 2));
        assert_eq!(w.topics[0].terms, vec!["catalyst", "x"]);
        assert_eq!((w.topics[1].start, w.topics[1].end), (3, 5));
        assert_ne!(w.topics[1].name, "Part 2");
        assert_eq!(w.topics[1].unit, "");
        assert_eq!(w.topics[1].difficulty, 5.0);
        assert!(normalize_window(&json!({ "nope": 1 }), 1, 5, &paras, "L").is_err());
    }

    #[test]
    fn windows_merge_and_units_fill() {
        let paras = lesson();
        let t = |s, e, name: &str, cont| WinTopic { start: s, end: e, from: String::new(), name: name.into(), unit: String::new(), summary: "S.".into(), difficulty: 3.0, terms: vec![], prereqs: vec![], continues: cont };
        let wins = vec![
            WinResult { title: None, topics: vec![t(1, 2, "Enzymes as catalysts", false)] },
            WinResult { title: None, topics: vec![t(3, 3, "Still catalysts", true), t(4, 5, "Allosteric regulation", false)] },
        ];
        let f = finalize_windows(&wins, &paras, "Lesson");
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].t.start, f[0].t.end), (1, 3));
        assert_eq!(f[0].t.unit, "Lesson");
        assert!(f[0].words > 100);
    }

    #[test]
    fn excerpts_cut_at_the_quote() {
        let paras = to_paras("First topic text runs here. And then the second idea begins mid paragraph and carries on.\n\nThe second idea continues in its own paragraph.");
        let ex = topic_excerpts(&paras, &[
            ExcerptTopic { start: 1, end: 1, from: "First topic text" },
            ExcerptTopic { start: 2, end: 2, from: "and then the second idea begins" },
        ]);
        assert_eq!(ex[0], "First topic text runs here.");
        assert!(ex[1].starts_with("And then the second idea begins mid paragraph"));
        assert!(ex[1].ends_with("its own paragraph."));
    }

    #[test]
    fn similar_neighbours_are_flagged() {
        let ts = vec![("Enzyme kinetics basics".to_string(), "U".to_string()), ("Enzyme kinetics models".to_string(), "U".to_string()), ("Photosynthesis".to_string(), "U".to_string())];
        assert_eq!(ambiguous_pairs(&ts), vec![(0, 1)]);
        assert_eq!(cache_key("quiz", "abc", "m", false), cache_key("quiz", "abc", "other", false));
        assert_ne!(cache_key("quiz", "abc", "m", true), cache_key("quiz", "abc", "other", true));
    }
}
