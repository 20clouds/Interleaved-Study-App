// Small helpers used across the core: time, day keys, word counts, names, natural ordering.
use jiff::{civil::Date, tz::TimeZone, Span, Timestamp, ZonedDateTime};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::sync::OnceLock;

pub const DAY: f64 = 86_400_000.0;

pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

pub fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub fn sha256(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn local(t: f64) -> Option<ZonedDateTime> {
    Timestamp::from_millisecond(t as i64).ok().map(|ts| ts.to_zoned_datetime(TimeZone::system()))
}

fn ms(z: &ZonedDateTime) -> f64 {
    z.timestamp().as_millisecond() as f64
}

/// Local calendar day "YYYY-MM-DD" for a timestamp.
pub fn day_key(t: f64) -> String {
    local(t).map_or_else(|| "1970-01-01".into(), |z| z.date().to_string())
}

pub fn today() -> String {
    day_key(now_ms())
}

/// The start of a day key's local day, in ms: midnight, or the first moment after it when a clock change skips midnight.
pub fn parse_day_key(k: &str) -> f64 {
    let d = Date::strptime("%Y-%m-%d", k).unwrap_or(Date::constant(1970, 1, 1));
    d.to_zoned_datetime(TimeZone::system()).map_or(0.0, |z| ms(&z))
}

/// Adds time to a timestamp. Months follow the calendar (Jan 31 + 1 month = Feb 28/29).
pub fn add_time(t: f64, amount: f64, unit: &str) -> f64 {
    match unit {
        "minutes" => t + amount * 60_000.0,
        "hours" => t + amount * 3_600_000.0,
        "days" => t + amount * DAY,
        // try_months, not months(): a count past jiff's limit (about 20,000 years) is refused instead of panicking.
        _ => local(t).and_then(|z| z.checked_add(Span::new().try_months(amount.round() as i64).ok()?).ok()).map_or(t, |z| ms(&z)),
    }
}

pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

/// JavaScript's Math.round (halves go up).
pub fn jround(v: f64) -> f64 {
    (v + 0.5).floor()
}

pub fn round1(v: f64) -> f64 {
    jround(v * 10.0) / 10.0
}

pub fn round5(v: f64) -> f64 {
    jround(v / 5.0) * 5.0
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("valid regex"))
}

pub fn word_count(s: &str) -> i64 {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"[\p{L}\p{N}][\p{L}\p{N}'’-]*").find_iter(s).count() as i64
}

/// True for names like "Part 3", "Section 2", "Chunk 7" which must never be used as topic names.
pub fn is_generic_name(name: &str) -> bool {
    static G: OnceLock<Regex> = OnceLock::new();
    static N: OnceLock<Regex> = OnceLock::new();
    let n = name.trim();
    n.is_empty()
        || re(&G, r"(?i)^(part|section|chunk|segment|piece|block|topic|unit|lesson|page)\s*[#.:-]?\s*(\d+|[ivxlc]+)\s*[.:)-]?$").is_match(n)
        || re(&N, r"^\d+[.)]?$").is_match(n)
}

/// Baseline study minutes from word count, adjusted by AI difficulty (1..5) when known.
pub fn suggested_minutes(words: i64, difficulty: Option<f64>, pace: f64) -> f64 {
    let pace = if pace > 0.0 { pace } else { 120.0 };
    let base = clamp(round5(words as f64 / pace), 10.0, 60.0);
    match difficulty {
        Some(d) if d != 0.0 => clamp(round5(base * (0.8 + 0.1 * clamp(d, 1.0, 5.0))), 10.0, 60.0),
        _ => base,
    }
}

pub fn norm_name(s: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"[^\p{L}\p{N}]+").replace_all(&s.to_lowercase(), " ").trim().to_string()
}

const NOT_CODES: &[&str] = &[
    "WEEK", "UNIT", "PART", "LEC", "LECT", "CHAP", "CH", "DAY", "QUIZ", "TEST", "EXAM", "LAB", "PAGE", "SEC", "MOD", "SET", "REV", "TERM",
    "YEAR", "ROOM", "NOTE", "NOTES", "HW", "PS", "FALL", "WINTER", "SPRING", "JAN", "FEB", "MAR", "MARCH", "APR", "APRIL", "MAY", "JUN",
    "JUNE", "JUL", "JULY", "AUG", "SEP", "SEPT", "OCT", "NOV", "DEC",
];

/// The course code a file name starts with ("CHM110 - Oct 9" → "CHM110", "bio 201_notes" → "BIO201"), else None.
pub fn course_code(file_name: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    let t = file_name.trim();
    let m = re(&R, r"^([A-Za-z]{2,5})[\s_-]?(\d{3,4}(?:[A-Za-z]\d?)?)").captures(t)?;
    let whole = m.get(0)?;
    // The code must end there: the regex engine has no lookahead, so try the shorter forms it would backtrack to.
    let letters = m.get(1)?.as_str();
    let digits_full = m.get(2)?.as_str();
    let start2 = m.get(2)?.start();
    let ok_end = |end: usize| !t[end..].chars().next().map(|c| c.is_ascii_alphanumeric()).unwrap_or(false);
    let mut pick: Option<&str> = None;
    if ok_end(whole.end()) {
        pick = Some(digits_full);
    } else {
        // Candidates in the order a backtracking engine tries them: longest first.
        let bytes: Vec<char> = digits_full.chars().collect();
        let nd = bytes.iter().take_while(|c| c.is_ascii_digit()).count();
        let mut cands: Vec<usize> = Vec::new();
        for d in (3..=nd.min(4)).rev() {
            if bytes.len() >= d + 2 && bytes[d].is_ascii_alphabetic() && bytes[d + 1].is_ascii_digit() { cands.push(d + 2) }
            if bytes.len() >= d + 1 && bytes[d].is_ascii_alphabetic() { cands.push(d + 1) }
            cands.push(d);
        }
        for len in cands {
            if len <= digits_full.len() && ok_end(start2 + len) { pick = Some(&digits_full[..len]); break }
        }
    }
    let digits = pick?;
    if NOT_CODES.contains(&letters.to_uppercase().as_str()) { return None }
    Some(format!("{letters}{digits}").to_uppercase())
}

/// Case-insensitive ordering where runs of digits compare as numbers ("Week 2" before "Week 10").
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b): (Vec<char>, Vec<char>) = (a.to_lowercase().chars().collect(), b.to_lowercase().chars().collect());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() { i += 1 }
            while j < b.len() && b[j].is_ascii_digit() { j += 1 }
            let na: String = a[si..i].iter().collect::<String>().trim_start_matches('0').to_string();
            let nb: String = b[sj..j].iter().collect::<String>().trim_start_matches('0').to_string();
            let o = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if o != Ordering::Equal { return o }
        } else {
            if a[i] != b[j] { return a[i].cmp(&b[j]) }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// The first `n` characters of a string (never splits a character).
pub fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

pub fn fmt_clock(ms: f64) -> String {
    let s = (ms / 1000.0).ceil().max(0.0) as i64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn course_codes() {
        assert_eq!(course_code("CHM110 - Oct 9.md").as_deref(), Some("CHM110"));
        assert_eq!(course_code("bio 201_notes").as_deref(), Some("BIO201"));
        assert_eq!(course_code("Week 101 notes"), None);
        assert_eq!(course_code("MAT1341A lecture").as_deref(), Some("MAT1341A"));
        assert_eq!(course_code("notes"), None);
        assert_eq!(course_code("abc12345"), None);
    }

    #[test]
    fn names_and_words() {
        assert!(is_generic_name("Part 3"));
        assert!(is_generic_name("section IV."));
        assert!(!is_generic_name("Enzyme kinetics"));
        assert_eq!(word_count("It's a well-known fact, isn't it?"), 6);
        assert_eq!(norm_name("  Michaelis–Menten: Kinetics! "), "michaelis menten kinetics");
        assert_eq!(suggested_minutes(1200, None, 120.0), 10.0);
        assert_eq!(suggested_minutes(3600, Some(5.0), 120.0), 40.0);
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["Week 10", "week 2", "Week 1"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["Week 1", "week 2", "Week 10"]);
    }

    #[test]
    fn months_follow_the_calendar() {
        let jan31 = parse_day_key("2026-01-31");
        assert_eq!(day_key(add_time(jan31, 1.0, "months")), "2026-02-28");
        assert_eq!(day_key(add_time(jan31, 1.0, "days")), "2026-02-01");
        // Past jiff's month limit the time is left as it was rather than panicking.
        for n in [1e9, -1e9, f64::INFINITY, f64::NAN] { assert_eq!(add_time(jan31, n, "months"), jan31) }
    }
}
