// Local, zero-token preprocessing: normalize text, strip junk, number paragraphs, detect headings,
// build AI windows and provisional topics (never "Part N").
use crate::types::{Heading, Para};
use crate::util::{is_generic_name, norm_name, word_count};
use std::collections::HashMap;

/// A regex compiled once, where it is used.
#[macro_export]
macro_rules! rx {
    ($pat:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($pat).expect("valid regex"))
    }};
}

pub fn normalize_text(raw: &str) -> String {
    let s = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    let s = rx!(r"\r\n?").replace_all(s, "\n");
    let s = rx!("[\u{00A0}\u{2007}\u{202F}]").replace_all(&s, " ");
    let s = rx!("[\u{00AD}\u{200B}-\u{200D}\u{2060}]").replace_all(&s, "");
    let s = rx!(r"(\p{L})-\n(\p{Ll})").replace_all(&s, "$1$2"); // de-hyphenate words broken across lines
    let s = s.replace('\t', "  ");
    let s = rx!(r"[^\S\n]{2,}").replace_all(&s, " "); // any run of non-newline whitespace becomes one space
    let s = s.split('\n').map(|l| l.trim_end()).collect::<Vec<_>>().join("\n");
    rx!(r"\n{3,}").replace_all(&s, "\n\n").trim().to_string()
}

fn digits_key(t: &str) -> String {
    rx!(r"\d+").replace_all(&t.to_lowercase(), "#").to_string()
}

/// Remove page numbers, horizontal rules and short lines repeated many times (running headers/footers).
pub fn strip_junk(text: &str) -> String {
    let page_num = rx!(r"(?i)^\s*(?:[-–—]\s*)?(?:page|p\.|pg\.?|seite|pagina)?\s*\d{1,4}\s*(?:(?:of|/|von|de)\s*\d{1,4})?\s*(?:[-–—])?\s*$");
    let rule = rx!(r"^\s*([-*_=]\s*){3,}$");
    let image = rx!(r"^!\[[^\]]*\]\([^)]*\)$");
    let md_head = rx!(r"^#{1,6}\s");
    let list = rx!(r"^[-*+]\s|^\d+[.)]\s");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for l in &lines {
        let k = digits_key(l.trim());
        let n = k.chars().count();
        if n > 0 && n < 80 { *counts.entry(k).or_insert(0) += 1; }
    }
    let mut out: Vec<&str> = Vec::new();
    for l in &lines {
        let t = l.trim();
        let n = t.chars().count();
        if n > 0 && n <= 40 && page_num.is_match(t) { continue } // short lines only
        if rule.is_match(t) { continue }
        if image.is_match(t) { continue } // image-only lines carry no study text
        if n > 0 && n < 80 && counts.get(&digits_key(t)).copied().unwrap_or(0) >= 3 && !md_head.is_match(t) && !list.is_match(t) { continue }
        out.push(l);
    }
    rx!(r"\n{3,}").replace_all(&out.join("\n"), "\n\n").trim().to_string()
}

fn strip_inline(s: &str) -> String {
    let s = rx!(r"!\[[^\[\]]*\]\([^()]*\)").replace_all(s, "");
    let s = rx!(r"\[([^\[\]]+)\]\([^()]*\)").replace_all(&s, "$1");
    let s = rx!(r"[*_`~]+").replace_all(&s, "");
    let s = rx!(r"<[^<>]+>").replace_all(&s, "");
    rx!(r"\s+").replace_all(&s, " ").trim().to_string()
}

fn starts_upper(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_uppercase())
}

/// Detect a heading line. Returns level 1..6 and clean text, or None.
pub fn detect_heading(block: &str) -> Option<Heading> {
    if block.contains('\n') {
        let lines: Vec<&str> = block.split('\n').collect();
        if lines.len() == 2 && !lines[0].trim().is_empty() {
            let b = lines[1].trim();
            if !b.is_empty() && b.chars().all(|c| c == '=') { return Some(Heading { level: 1, text: strip_inline(lines[0]) }) }
            if !b.is_empty() && b.chars().all(|c| c == '-') { return Some(Heading { level: 2, text: strip_inline(lines[0]) }) }
        }
        return None;
    }
    let t = block.trim();
    let len = t.chars().count();
    if len <= 300 {
        if let Some(m) = rx!(r"^(#{1,6})\s+(.+?)\s*#*$").captures(t) {
            return Some(Heading { level: m[1].len() as i64, text: strip_inline(&m[2]) });
        }
    }
    if let Some(m) = rx!(r"^(?:\*\*|__)(.{2,90})(?:\*\*|__):?$").captures(t) {
        if word_count(&m[1]) <= 12 { return Some(Heading { level: 3, text: strip_inline(&m[1]) }) }
    }
    let wc = word_count(t);
    if len > 90 || wc > 12 || wc == 0 { return None }
    if rx!(r"[.,;:!?]$").is_match(t) && !rx!(r"^\d+(\.\d+)*\.?\s").is_match(t) { return None }
    if rx!(r"^[-*+>|]").is_match(t) { return None }
    if let Some(m) = rx!(r"^((?:\d+\.)+\d*|\d+\)?|[IVXLC]+\.)\s+(\S.*)$").captures(t) {
        if starts_upper(&m[2]) {
            let depth = m[1].trim_end_matches('.').split('.').count() as i64;
            return Some(Heading { level: (depth + 1).min(6), text: strip_inline(t) });
        }
    }
    let letters: String = t.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.chars().count() >= 4 && letters == letters.to_uppercase() && letters.chars().any(|c| c.is_uppercase()) {
        return Some(Heading { level: 2, text: strip_inline(t) });
    }
    let words: Vec<&str> = t.split_whitespace().collect();
    let caps = words.iter().filter(|w| starts_upper(w)).count();
    if words.len() >= 2 && words.len() <= 9 && caps as f64 / words.len() as f64 >= 0.6 && !rx!(r"[.!?]$").is_match(t) {
        return Some(Heading { level: 3, text: strip_inline(t) });
    }
    None
}

const MAX_PARA_WORDS: i64 = 350;

fn split_long(text: &str) -> Vec<String> {
    if word_count(text) <= MAX_PARA_WORDS { return vec![text.to_string()] }
    let sentences: Vec<&str> = rx!(r#"[^.!?]+[.!?]+(?:["')\]]+)?\s*|[^.!?]+$"#).find_iter(text).map(|m| m.as_str()).collect();
    let sentences = if sentences.is_empty() { vec![text] } else { sentences };
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for s in sentences {
        if !cur.is_empty() && word_count(&cur) + word_count(s) > 200 {
            out.push(cur.trim().to_string());
            cur.clear();
        }
        cur.push_str(s);
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

fn para(text: &str, heading: Option<Heading>) -> Para {
    Para { id: 0, text: text.to_string(), words: word_count(text), heading }
}

/// Split normalized text into numbered paragraphs (headings are their own paragraphs).
pub fn to_paras(text: &str) -> Vec<Para> {
    let mut out: Vec<Para> = Vec::new();
    for b in rx!(r"\n\s*\n").split(text).map(|b| b.trim()).filter(|b| !b.is_empty()) {
        // A heading line immediately followed by body text in the same block
        let lines: Vec<&str> = b.split('\n').collect();
        let first_h = if lines.len() > 1 { detect_heading(lines[0]) } else { None };
        let setext = lines.len() == 2 && rx!(r"^(=+|-+)$").is_match(lines[1].trim());
        if let (Some(h), false) = (first_h, setext) {
            out.push(para(lines[0].trim(), Some(h)));
            let rest = lines[1..].join("\n");
            let rest = rest.trim();
            if !rest.is_empty() { for piece in split_long(rest) { out.push(para(&piece, None)); } }
            continue;
        }
        if let Some(h) = detect_heading(b) {
            out.push(para(b, Some(h)));
            continue;
        }
        for piece in split_long(b) { out.push(para(&piece, None)); }
    }
    for (i, p) in out.iter_mut().enumerate() { p.id = i as i64 + 1; }
    out
}

pub struct Prepared {
    pub text: String,
    pub paras: Vec<Para>,
    pub words: i64,
    pub outline: String,
}

pub fn prepare(raw: &str) -> Prepared {
    let text = strip_junk(&normalize_text(raw));
    let paras = to_paras(&text);
    let words = paras.iter().map(|p| p.words).sum();
    let outline = build_outline(&paras, 1500);
    Prepared { text, paras, words, outline }
}

pub fn build_outline(paras: &[Para], max_chars: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut len = 0;
    for p in paras {
        let Some(h) = &p.heading else { continue };
        let l = format!("{}{} [p{}]", "  ".repeat((h.level - 1).max(0) as usize), h.text, p.id);
        let n = l.chars().count();
        if len + n > max_chars { lines.push("…".into()); break }
        len += n + 1;
        lines.push(l);
    }
    lines.join("\n")
}

pub fn est_tokens(chars: usize) -> f64 {
    (chars as f64 / 4.0).ceil()
}

#[derive(Clone, Debug)]
pub struct Window {
    pub index: usize,
    pub from: i64,
    pub to: i64,
    pub tokens: f64,
}

/// Group paragraphs into windows of about `target` tokens, breaking at paragraph (preferably heading) boundaries.
pub fn build_windows(paras: &[Para], target: f64) -> Vec<Window> {
    let mut wins: Vec<Window> = Vec::new();
    let mut from: Option<usize> = None;
    let mut tokens = 0.0;
    for (i, p) in paras.iter().enumerate() {
        let t = est_tokens(p.text.chars().count() + 8);
        if let Some(f) = from {
            let at_heading = p.heading.is_some() && tokens >= target * 0.7;
            if tokens + t > target || at_heading {
                wins.push(Window { index: wins.len(), from: paras[f].id, to: paras[i - 1].id, tokens });
                from = None;
                tokens = 0.0;
            }
        }
        if from.is_none() { from = Some(i); }
        tokens += t;
    }
    if let Some(f) = from { wins.push(Window { index: wins.len(), from: paras[f].id, to: paras[paras.len() - 1].id, tokens }); }
    // Fold a tiny trailing window into the previous one
    if wins.len() >= 2 && wins[wins.len() - 1].tokens < target * 0.25 {
        let last = wins.pop().unwrap();
        let prev = wins.last_mut().unwrap();
        prev.to = last.to;
        prev.tokens += last.tokens;
    }
    wins
}

const STOP_TAIL: [&str; 19] = ["a", "an", "the", "of", "and", "or", "to", "in", "on", "for", "with", "by", "is", "are", "as", "at", "from", "that", "which"];

/// About six meaningful words from the first sentence of a section.
pub fn first_sentence_name(text: &str) -> String {
    let no_marks = rx!(r"(?m)^#{1,6}\s+").replace_all(text, "");
    let clean = strip_inline(&no_marks);
    let clean = rx!(r"(?i)^(in this (section|chapter|lesson|part|unit)|this (section|chapter|lesson)|here we|we will|we now|now we|let us|let's|first,|second,|finally,|next,|in summary,|overall,)\s*").replace(&clean, "");
    let sentence = rx!(r"^[^.!?:;]+").find(&clean).map(|m| m.as_str()).unwrap_or(&clean);
    let mut words: Vec<&str> = sentence.split_whitespace().filter(|w| w.chars().any(|c| c.is_alphanumeric())).take(6).collect();
    while words.len() > 2 && STOP_TAIL.contains(&words[words.len() - 1].to_lowercase().as_str()) { words.pop(); }
    let joined = words.join(" ");
    let s = joined.trim_end_matches(|c: char| ",()\"'“”".contains(c)).trim();
    let mut chars = s.chars();
    match chars.next() {
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Ensure a name is never generic; fall back to the first sentence, then to the lesson title.
pub fn safe_name(candidate: &str, body_text: &str, lesson_title: &str) -> String {
    let c = rx!(r"\s+").replace_all(candidate.trim(), " ").to_string();
    if !c.is_empty() && !is_generic_name(&c) { return c }
    let fs = first_sentence_name(body_text);
    if !fs.is_empty() && !is_generic_name(&fs) { return fs }
    format!("Notes from {lesson_title}")
}

#[derive(Clone, Debug)]
pub struct ProvisionalTopic {
    pub name: String,
    pub unit: String,
    pub start: i64,
    pub end: i64,
    pub words: i64,
}

struct Section {
    heading: Option<String>,
    unit: String,
    paras: Vec<Para>,
}

/// Provisional topics before AI: sections split at headings, big sections split by size and named from their first sentence.
pub fn provisional_topics(paras: &[Para], lesson_title: &str) -> Vec<ProvisionalTopic> {
    let max_words = 1200i64;
    if paras.is_empty() { return vec![] }
    let levels: Vec<i64> = paras.iter().filter_map(|p| p.heading.as_ref().map(|h| h.level)).collect();
    let min_level = levels.iter().copied().min().unwrap_or(1);
    let many_top = levels.iter().filter(|l| **l == min_level).count() > 1;
    // Unit = nearest top-level heading; topic boundary = any heading
    let mut sections: Vec<Section> = Vec::new();
    let mut unit = lesson_title.to_string();
    for p in paras {
        if let Some(h) = &p.heading {
            if h.level == min_level && many_top { unit = h.text.clone(); }
            sections.push(Section { heading: Some(h.text.clone()), unit: unit.clone(), paras: vec![p.clone()] });
        } else {
            if sections.is_empty() { sections.push(Section { heading: None, unit: unit.clone(), paras: vec![] }); }
            sections.last_mut().unwrap().paras.push(p.clone());
        }
    }
    // Merge heading-only or tiny sections into the following one (or previous for the last)
    let body_words = |s: &Section| s.paras.iter().filter(|p| p.heading.is_none()).map(|p| p.words).sum::<i64>();
    let mut merged: Vec<Section> = Vec::new();
    let n = sections.len();
    let mut carry: Option<Section> = None;
    for (i, mut s) in sections.into_iter().enumerate() {
        if let Some(prev) = carry.take() {
            let w = body_words(&prev);
            let mut ps = prev.paras;
            ps.append(&mut s.paras);
            s.paras = ps;
            if s.heading.is_none() || w == 0 { s.heading = s.heading.or(prev.heading); }
        }
        let w = body_words(&s);
        if w < 60 && i < n - 1 { carry = Some(s); continue }
        if w < 60 && !merged.is_empty() { merged.last_mut().unwrap().paras.extend(s.paras); continue }
        merged.push(s);
    }
    let mut out: Vec<ProvisionalTopic> = Vec::new();
    for s in &merged {
        let mut pieces: Vec<Vec<&Para>> = Vec::new();
        let mut cur: Vec<&Para> = Vec::new();
        let mut w = 0i64;
        for p in &s.paras {
            if w + p.words > max_words && !cur.is_empty() && (w as f64) > max_words as f64 * 0.4 {
                pieces.push(std::mem::take(&mut cur));
                w = 0;
            }
            cur.push(p);
            w += p.words;
        }
        if !cur.is_empty() { pieces.push(cur); }
        for (idx, pp) in pieces.iter().enumerate() {
            let mut body = pp.iter().filter(|p| p.heading.is_none()).map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n");
            if body.is_empty() { body = pp.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n"); }
            let base = match (&s.heading, idx) { (Some(h), 0) => h.clone(), _ => first_sentence_name(&body) };
            out.push(ProvisionalTopic {
                name: safe_name(&base, &body, lesson_title),
                unit: if !s.unit.is_empty() && !is_generic_name(&s.unit) { s.unit.clone() } else { lesson_title.to_string() },
                start: pp[0].id,
                end: pp[pp.len() - 1].id,
                words: pp.iter().map(|p| p.words).sum(),
            });
        }
    }
    // Disambiguate identical names within a lesson
    let mut seen: HashMap<String, usize> = HashMap::new();
    for t in out.iter_mut() {
        let k = norm_name(&t.name);
        let n = seen.entry(k.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            let p = paras.iter().find(|x| x.id > t.start && x.heading.is_none()).or_else(|| paras.iter().find(|x| x.id == t.start));
            if let Some(p) = p {
                let alt = first_sentence_name(&p.text);
                if !alt.is_empty() && norm_name(&alt) != k { t.name = alt; }
            }
        }
    }
    out
}

pub fn slice_text(paras: &[Para], start: i64, end: i64) -> String {
    paras.iter().filter(|p| p.id >= start && p.id <= end).map(|p| p.text.as_str()).collect::<Vec<_>>().join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_strips() {
        let t = normalize_text("\u{FEFF}Line one\r\nhyphen-\nated  word\u{00A0}here\n\n\n\nnext");
        assert_eq!(t, "Line one\nhyphenated word here\n\nnext");
        let j = strip_junk("Title\n\nPage 3 of 10\n\nBody text here.\n\n---\n\n![img](x.png)\n\n12");
        assert_eq!(j, "Title\n\nBody text here.");
    }

    #[test]
    fn headings() {
        assert_eq!(detect_heading("## Enzyme kinetics").unwrap().level, 2);
        assert_eq!(detect_heading("**Key Terms**").unwrap().level, 3);
        assert_eq!(detect_heading("2.1 Reaction Rates").unwrap().level, 3);
        assert_eq!(detect_heading("CELL STRUCTURE").unwrap().level, 2);
        assert_eq!(detect_heading("Title\n=====").unwrap().level, 1);
        assert!(detect_heading("This is a plain sentence that ends with a period.").is_none());
        assert!(detect_heading("- a list item").is_none());
    }

    #[test]
    fn paragraphs_windows_and_names() {
        let body = "Enzymes speed up reactions by lowering activation energy. ".repeat(30);
        let text = format!("# Enzymes\n\n{body}\n\n# Inhibition\n\n{body}");
        let p = prepare(&text);
        assert_eq!(p.paras.len(), 4);
        assert!(p.paras[0].heading.is_some());
        assert_eq!(p.outline, "Enzymes [p1]\nInhibition [p3]");
        let wins = build_windows(&p.paras, 3500.0);
        assert_eq!(wins.len(), 1);
        assert_eq!((wins[0].from, wins[0].to), (1, 4));
        let small = build_windows(&p.paras, 400.0);
        assert!(small.len() >= 2);
        let prov = provisional_topics(&p.paras, "Lesson");
        assert_eq!(prov.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["Enzymes", "Inhibition"]);
        assert_eq!(prov[0].unit, "Enzymes");
        assert_eq!(first_sentence_name("In this section we look at the citric acid cycle. More."), "We look at the citric acid");
        assert_eq!(safe_name("Part 2", "Glycolysis splits glucose in two. Then...", "L1"), "Glycolysis splits glucose in two");
    }

    #[test]
    fn long_paragraphs_are_split() {
        let long = "This sentence has exactly seven words total. ".repeat(80);
        let ps = to_paras(long.trim());
        assert!(ps.len() >= 2);
        assert!(ps.iter().all(|p| p.words <= 210));
    }
}
