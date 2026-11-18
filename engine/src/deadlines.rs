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

/// Spreadsheet limits. A course calendar is a few hundred rows; these only stop a hostile file from taking the app
/// down. Sheet XML is streamed, so what has to be bounded is the file itself, everything it unzips to (shared strings
/// are held in memory) and the cells kept.
const MAX_SHEET_FILE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_SHEET_UNZIPPED_BYTES: u64 = 200 * 1024 * 1024;
const MAX_SHEETS: usize = 50;
const MAX_ROWS: usize = 5000;
const MAX_COLS: u32 = 200;

/// The file as one buffer, read once: only a regular file, never more than the limit.
fn read_sheet_file(file: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let too_big = || format!("This spreadsheet is larger than {} MB.", MAX_SHEET_FILE_BYTES / 1_048_576);
    let md = std::fs::metadata(file).map_err(|e| format!("Could not read the file: {e}"))?;
    if !md.is_file() { return Err("This is not a regular file.".into()) }
    if md.len() > MAX_SHEET_FILE_BYTES { return Err(too_big()) }
    let mut buf = Vec::with_capacity(md.len() as usize);
    std::fs::File::open(file).and_then(|f| f.take(MAX_SHEET_FILE_BYTES + 1).read_to_end(&mut buf)).map_err(|e| format!("Could not read the file: {e}"))?;
    if buf.len() as u64 > MAX_SHEET_FILE_BYTES { return Err(too_big()) }
    Ok(buf)
}

/// Whether an .xlsx unzips to a sane size. The zip reader refuses any entry that inflates past its declared size, so
/// the declared sizes are a real bound on what calamine can be made to hold.
fn unzipped_ok(bytes: &[u8]) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "This .xlsx file could not be opened (it may be damaged or password-protected).".to_string())?;
    let mut total: u64 = 0;
    for i in 0..zip.len() {
        let size = zip.by_index_raw(i).map_err(|_| "This .xlsx file is damaged.".to_string())?.size();
        total = total.saturating_add(size);
        if total > MAX_SHEET_UNZIPPED_BYTES { return Err("This spreadsheet unpacks to more data than a deadline list could hold.".into()) }
    }
    Ok(())
}

/// Rows keyed by sheet row, each a map from column to value; at most MAX_ROWS rows (the topmost) and MAX_COLS columns.
#[derive(Default)]
struct SparseRows(std::collections::BTreeMap<u32, std::collections::BTreeMap<u32, Value>>);

impl SparseRows {
    fn put(&mut self, row: u32, col: u32, v: Value) {
        if v.is_null() || col >= MAX_COLS { return }
        if !self.0.contains_key(&row) && self.0.len() >= MAX_ROWS {
            // Cells normally arrive row by row, so a full map only takes rows above the ones it has.
            match self.0.last_key_value() { Some((&last, _)) if row < last => { self.0.remove(&last); } _ => return }
        }
        self.0.entry(row).or_default().insert(col, v);
    }

    /// Rows as the table shows them: column A first, empty cells as null.
    fn into_rows(self) -> Vec<Vec<Value>> {
        self.0.into_values().map(|cells| {
            let width = cells.keys().next_back().map_or(0, |c| *c as usize + 1);
            let mut row = vec![Value::Null; width];
            for (c, v) in cells { row[c as usize] = v; }
            row
        }).collect()
    }
}

fn cell_value(d: &calamine::DataRef) -> Value {
    use calamine::DataRef as D;
    let text = |s: &str| if s.trim().is_empty() { Value::Null } else { Value::String(s.to_string()) };
    match d {
        D::Empty | D::Error(_) => Value::Null,
        D::String(s) => text(s),
        D::SharedString(s) => text(s),
        D::Int(i) => json!(i),
        D::Float(f) => if f.fract() == 0.0 && f.abs() < 1e15 { json!(*f as i64) } else { json!(f) },
        D::Bool(b) => json!(b),
        D::DateTime(d) => serial_to_iso(d.as_f64()).map(Value::String).unwrap_or(Value::Null),
        D::DateTimeIso(s) => Value::String(take_chars(s, 10)),
        D::DurationIso(s) => Value::String(s.clone()),
    }
}

/// Read every sheet of an .xlsx or .csv file as rows (at most 5000 each); dates become YYYY-MM-DD.
/// Cells are streamed, never laid out as one grid: calamine's grid spans the used range, so two cells at A1 and
/// XFD1048576 in a few-kilobyte file would ask for billions of cells and abort the app. The legacy .xls format
/// can only be read as such a grid, so it is not accepted.
pub fn read_sheets(file: &Path) -> Result<Vec<Sheet>, String> {
    use calamine::Reader;
    let ext = file.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext != "csv" && ext != "xlsx" { return Err("Use an .xlsx or .csv file. Save older .xls files as .xlsx first.".into()) }
    let bytes = read_sheet_file(file)?;
    if ext == "csv" {
        let mut rd = csv::ReaderBuilder::new().has_headers(false).flexible(true).from_reader(bytes.as_slice());
        let mut rows = SparseRows::default();
        for (r, rec) in rd.records().take(MAX_ROWS).enumerate() {
            let rec = rec.map_err(|e| format!("Could not read the file: {e}"))?;
            for (c, cell) in rec.iter().enumerate() { rows.put(r as u32, c as u32, if cell.trim().is_empty() { Value::Null } else { Value::String(cell.to_string()) }); }
        }
        let name = file.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Sheet1".into());
        return Ok(vec![Sheet { name, rows: rows.into_rows() }]);
    }
    unzipped_ok(&bytes)?;
    let mut wb = calamine::Xlsx::new(std::io::Cursor::new(bytes)).map_err(|e| format!("Could not read the spreadsheet: {e}"))?;
    let mut out = vec![];
    for name in wb.sheet_names().into_iter().take(MAX_SHEETS) {
        let Ok(mut cells) = wb.worksheet_cells_reader(&name) else { continue };
        let mut rows = SparseRows::default();
        while let Some(cell) = cells.next_cell().map_err(|e| format!("Could not read the spreadsheet: {e}"))? {
            let (r, c) = cell.get_position();
            rows.put(r, c, cell_value(cell.get_value()));
        }
        out.push(Sheet { name, rows: rows.into_rows() });
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

    /// A minimal .xlsx whose one sheet holds the given `<c>` cells (inline strings).
    fn xlsx(cells: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opt = zip::write::SimpleFileOptions::default();
        let mut file = |name: &str, body: &str| { z.start_file(name, opt).unwrap(); z.write_all(body.as_bytes()).unwrap(); };
        file("[Content_Types].xml", r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#);
        file("_rels/.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#);
        file("xl/workbook.xml", r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Plan" sheetId="1" r:id="rId1"/></sheets></workbook>"#);
        file("xl/_rels/workbook.xml.rels", r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#);
        let mut rows = String::new();
        for (r, v) in cells {
            let n: String = r.chars().filter(|c| c.is_ascii_digit()).collect();
            rows.push_str(&format!(r#"<row r="{n}"><c r="{r}" t="inlineStr"><is><t>{v}</t></is></c></row>"#));
        }
        file("xl/worksheets/sheet1.xml", &format!(r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>{rows}</sheetData></worksheet>"#));
        z.finish().unwrap().into_inner()
    }

    fn read_bytes(name: &str, bytes: &[u8]) -> Result<Vec<Sheet>, String> {
        let dir = std::env::temp_dir().join(format!("is-sheets-{}", crate::util::uid()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(name);
        std::fs::write(&f, bytes).unwrap();
        let res = read_sheets(&f);
        let _ = std::fs::remove_dir_all(&dir);
        res
    }

    #[test]
    fn xlsx_rows_keep_their_columns() {
        let s = read_bytes("plan.xlsx", &xlsx(&[("B2", "Essay 1"), ("C2", "2026-03-04"), ("A4", "Exam")])).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "Plan");
        assert_eq!(s[0].rows, vec![vec![Value::Null, json!("Essay 1"), json!("2026-03-04")], vec![json!("Exam")]]);
    }

    #[test]
    fn far_apart_cells_do_not_make_a_grid() {
        // Laid out as one grid this is 17 billion cells; streamed it is two rows, the far column dropped.
        let s = read_bytes("bomb.xlsx", &xlsx(&[("A1", "Quiz"), ("XFD1048576", "x")])).unwrap();
        assert_eq!(s[0].rows, vec![vec![json!("Quiz")]]);
    }

    #[test]
    fn rows_are_capped_and_odd_files_refused() {
        let csv: String = (0..MAX_ROWS + 10).map(|i| format!("item {i},2026-01-01\n")).collect();
        let s = read_bytes("big.csv", csv.as_bytes()).unwrap();
        assert_eq!(s[0].rows.len(), MAX_ROWS);
        assert!(read_bytes("old.xls", b"\xD0\xCF\x11\xE0").is_err());
        assert!(read_bytes("fake.xlsx", b"not a zip").is_err());
        let mut many = SparseRows::default();
        for r in (0..MAX_ROWS as u32 + 5).rev() { many.put(r, 0, json!("x")); }
        assert_eq!(many.0.len(), MAX_ROWS);
        assert_eq!(many.0.keys().next_back(), Some(&(MAX_ROWS as u32 - 1)));
    }

    #[test]
    fn markdown_table() {
        let md = sheets_to_markdown(&[Sheet { name: "S".into(), rows: vec![vec![json!("a"), json!("b|c")], vec![json!(1), Value::Null]] }]);
        assert_eq!(md, "## S\n| a | b\\|c |\n| --- | --- |\n| 1 |  |\n");
        assert_eq!(serial_to_iso(46000.0).as_deref(), Some("2025-12-09"));
    }
}
