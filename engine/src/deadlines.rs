// Deadlines from spreadsheets: sheets are read into rows, turned into Markdown, cut into chunks that each keep
// their sheet heading and table header, and the model returns dated items as JSON.
use crate::aijson::{cap, snippet, InvalidJson};
use crate::rx;
use crate::types::ExtractedDeadline;
use crate::util::{norm_name, take_chars};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;

pub const MAX_SHEET_MARKDOWN: usize = 400_000;
pub const CHUNK_ROWS: usize = 60;

pub const DEADLINES_SYSTEM: &str = "You extract deadlines from a spreadsheet that was converted to Markdown (each sheet starts with a \"## Sheet name\" heading).
A deadline is any item with a due or event date: assignments, exams, quizzes, projects, presentations, readings due.
Return ONLY compact JSON, no prose, no code fences:
{\"deadlines\":[{\"title\":string,\"date\":\"YYYY-MM-DD\",\"course\":string|null,\"notes\":string|null}]}
Rules: one entry per dated item; skip rows without a date or without a clear item; never invent items or dates.
title: short and specific, at most 12 words, taken from the row (e.g. \"Essay 1\", \"Biology midterm\").
date: convert any format to YYYY-MM-DD and drop times. When the year is missing, pick the year that puts the date closest to today. For a range, use its end date. For ambiguous numeric dates like 03/04/2026, read day/month unless the same column shows month/day (a second number above 12).
course: from a course column, the sheet name or the row text; when it is one of the known courses, copy that exact spelling; otherwise null.
notes: other useful details from the same row (weight, time, room, format), or null. Never write \"NaN\" or \"None\".";

pub fn deadlines_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "deadlines": { "type": "array", "items": {
            "type": "object",
            "properties": { "title": { "type": "string" }, "date": { "type": "string", "format": "date" }, "course": { "type": ["string", "null"] }, "notes": { "type": ["string", "null"] } },
            "required": ["title", "date", "course", "notes"]
        } } },
        "required": ["deadlines"]
    })
}

pub fn deadline_key(title: &str, date: &str, course: Option<&str>) -> String {
    format!("{}|{}|{}", norm_name(title), date, norm_name(course.unwrap_or("")))
}

pub fn deadlines_prompt(chunk: &str, part: usize, of: usize, today: &str, courses: &[String]) -> String {
    let known = if courses.is_empty() { "(none yet)".to_string() } else { courses.join("; ") };
    let part = if of > 1 { format!(" part {part} of {of}") } else { String::new() };
    format!("Today is {today}.\nKnown courses: {known}\n\nSpreadsheet{part}:\n{chunk}")
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<Value>>,
}

fn cell_text(v: Option<&Value>) -> String {
    let s = match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(x) => x.to_string(),
    };
    rx!(r"\r?\n").replace_all(&s, " ").replace('|', "\\|").trim().to_string()
}

/// "## sheet" + a Markdown table per sheet.
pub fn sheets_to_markdown(sheets: &[Sheet]) -> String {
    let mut out: Vec<String> = vec![];
    for s in sheets {
        let rows: Vec<&Vec<Value>> = s.rows.iter().filter(|r| r.iter().any(|c| !cell_text(Some(c)).is_empty())).collect();
        if rows.is_empty() { continue }
        let width = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        let line = |r: &Vec<Value>| format!("| {} |", (0..width).map(|i| cell_text(r.get(i))).collect::<Vec<_>>().join(" | "));
        out.push(format!("## {}", s.name));
        out.push(line(rows[0]));
        out.push(format!("| {} |", vec!["---"; width].join(" | ")));
        for r in &rows[1..] { out.push(line(r)); }
        out.push(String::new());
    }
    out.join("\n")
}

fn is_separator(l: &str) -> bool {
    rx!(r"^\|(\s*:?-{3,}:?\s*\|)+\s*$").is_match(l.trim())
}

/// Split Markdown into chunks of at most `max_rows` table rows and about `max_chars` characters.
/// Every chunk starts with the current sheet heading and table header, so the model always sees column names.
pub fn chunk_markdown(md: &str, max_rows: usize, max_chars: usize) -> Vec<String> {
    let md = md.replace("\r\n", "\n");
    let lines: Vec<&str> = md.split('\n').collect();
    let mut chunks: Vec<String> = vec![];
    let mut heading = String::new();
    let mut header: Vec<String> = vec![];
    let mut body: Vec<String> = vec![];
    let (mut rows, mut chars) = (0usize, 0usize);
    fn flush(chunks: &mut Vec<String>, heading: &str, header: &[String], body: &mut Vec<String>, rows: &mut usize, chars: &mut usize) {
        if body.iter().any(|l| !l.trim().is_empty()) {
            let mut all: Vec<&str> = vec![];
            if !heading.is_empty() { all.push(heading); }
            all.extend(header.iter().map(|s| s.as_str()));
            all.extend(body.iter().map(|s| s.as_str()));
            chunks.push(all.join("\n").trim().to_string());
        }
        body.clear();
        *rows = 0;
        *chars = 0;
    }
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if rx!(r"^#{1,6}\s").is_match(l) {
            flush(&mut chunks, &heading, &header, &mut body, &mut rows, &mut chars);
            heading = l.to_string();
            header.clear();
        } else if l.trim().starts_with('|') && is_separator(lines.get(i + 1).copied().unwrap_or("")) {
            flush(&mut chunks, &heading, &header, &mut body, &mut rows, &mut chars);
            header = vec![l.to_string(), lines[i + 1].to_string()];
            i += 1;
        } else if l.trim().is_empty() {
            if !body.is_empty() { body.push(l.to_string()); }
        } else {
            if rows >= max_rows || chars + l.len() > max_chars { flush(&mut chunks, &heading, &header, &mut body, &mut rows, &mut chars); }
            body.push(l.to_string());
            rows += 1;
            chars += l.len() + 1;
        }
        i += 1;
    }
    flush(&mut chunks, &heading, &header, &mut body, &mut rows, &mut chars);
    chunks
}

fn real_date(s: &str) -> bool {
    s.len() == 10 && rx!(r"^\d{4}-\d{2}-\d{2}$").is_match(s) && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

fn clean(v: Option<&Value>) -> String {
    let s = v.and_then(|x| x.as_str()).unwrap_or("").trim();
    if rx!(r"(?i)^(nan|none|null|n/a)$").is_match(s) { String::new() } else { s.to_string() }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Extracted {
    pub items: Vec<ExtractedDeadline>,
    pub dropped: usize,
}

/// Validate one chunk's answer. Entries without a title or a real date are counted in `dropped`, not refused.
pub fn validate_deadlines(raw: &Value) -> Result<Extracted, InvalidJson> {
    let list = raw.as_array().or_else(|| raw.get("deadlines").and_then(|x| x.as_array()))
        .ok_or_else(|| InvalidJson(format!("Expected {{\"deadlines\":[...]}}: {}", snippet(raw, 300))))?;
    let mut out = Extracted::default();
    for x in list {
        let title = take_chars(&clean(x.get("title")), cap::NAME);
        let date = take_chars(&clean(x.get("date")), 10);
        if title.is_empty() || !real_date(&date) { out.dropped += 1; continue }
        let course = take_chars(&clean(x.get("course")), cap::COURSE);
        out.items.push(ExtractedDeadline { title, date, course_name: if course.is_empty() { None } else { Some(course) }, notes: take_chars(&clean(x.get("notes")), cap::NOTES) });
    }
    Ok(out)
}

/// Merge chunk results, dropping repeats (same title, date and course) that span chunks or sheets.
pub fn merge_extracted(parts: Vec<Extracted>) -> Extracted {
    let mut seen = HashSet::new();
    let mut out = Extracted::default();
    for p in parts {
        out.dropped += p.dropped;
        for it in p.items {
            if seen.insert(deadline_key(&it.title, &it.date, it.course_name.as_deref())) { out.items.push(it); }
        }
    }
    out.items.sort_by(|a, b| a.date.cmp(&b.date).then(a.title.cmp(&b.title)));
    out
}

fn serial_to_iso(n: f64) -> Option<String> {
    if !n.is_finite() || !(1.0..=2_958_465.0).contains(&n) { return None }
    let base = chrono::NaiveDate::from_ymd_opt(1899, 12, 30)?;
    Some((base + chrono::Duration::days(n.floor() as i64)).format("%Y-%m-%d").to_string())
}

/// Read every sheet of an .xlsx/.xls/.csv file as rows (at most 5000 each); dates become YYYY-MM-DD.
pub fn read_sheets(file: &Path) -> Result<Vec<Sheet>, String> {
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext == "csv" {
        let mut rd = csv::ReaderBuilder::new().has_headers(false).flexible(true).from_path(file).map_err(|e| format!("Could not read the file: {e}"))?;
        let mut rows = vec![];
        for rec in rd.records().take(5000) {
            let rec = rec.map_err(|e| format!("Could not read the file: {e}"))?;
            let row: Vec<Value> = rec.iter().map(|c| if c.trim().is_empty() { Value::Null } else { Value::String(c.to_string()) }).collect();
            if row.iter().any(|c| !c.is_null()) { rows.push(row); }
        }
        let name = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sheet1".into());
        return Ok(vec![Sheet { name, rows }]);
    }
    use calamine::{Data, Reader};
    let mut wb = calamine::open_workbook_auto(file).map_err(|e| format!("Could not read the spreadsheet: {e}"))?;
    let mut out = vec![];
    for name in wb.sheet_names().to_vec() {
        let Ok(range) = wb.worksheet_range(&name) else { continue };
        let pad = range.start().map(|(_, c)| c as usize).unwrap_or(0);
        let mut rows = vec![];
        for r in range.rows().take(5000) {
            let mut row: Vec<Value> = vec![Value::Null; pad];
            for c in r {
                row.push(match c {
                    Data::Empty | Data::Error(_) => Value::Null,
                    Data::String(s) => if s.trim().is_empty() { Value::Null } else { Value::String(s.clone()) },
                    Data::Int(i) => json!(i),
                    Data::Float(f) => if f.fract() == 0.0 && f.abs() < 1e15 { json!(*f as i64) } else { json!(f) },
                    Data::Bool(b) => json!(b),
                    Data::DateTime(d) => serial_to_iso(d.as_f64()).map(Value::String).unwrap_or(Value::Null),
                    Data::DateTimeIso(s) => Value::String(take_chars(s, 10)),
                    Data::DurationIso(s) => Value::String(s.clone()),
                });
            }
            if row.iter().any(|c| !c.is_null()) { rows.push(row); }
        }
        out.push(Sheet { name, rows });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_keep_heading_and_header() {
        let md = "## Fall\n| Item | Due |\n| --- | --- |\n| A | 1 |\n| B | 2 |\n| C | 3 |\n";
        let c = chunk_markdown(md, 2, 14000);
        assert_eq!(c.len(), 2);
        assert!(c[1].starts_with("## Fall\n| Item | Due |\n| --- | --- |\n| C | 3 |"));
    }

    #[test]
    fn validates_and_merges() {
        let v = validate_deadlines(&json!({ "deadlines": [
            { "title": "Essay 1", "date": "2026-03-04", "course": "None", "notes": null },
            { "title": "Bad", "date": "2026-02-30", "course": null, "notes": null }
        ] })).unwrap();
        assert_eq!((v.items.len(), v.dropped), (1, 1));
        assert!(v.items[0].course_name.is_none());
        let m = merge_extracted(vec![v.clone(), v]);
        assert_eq!((m.items.len(), m.dropped), (1, 2));
    }

    #[test]
    fn markdown_table() {
        let md = sheets_to_markdown(&[Sheet { name: "S".into(), rows: vec![vec![json!("a"), json!("b|c")], vec![json!(1), Value::Null]] }]);
        assert_eq!(md, "## S\n| a | b\\|c |\n| --- | --- |\n| 1 |  |\n");
        assert_eq!(serial_to_iso(46000.0).as_deref(), Some("2025-12-09"));
    }
}
