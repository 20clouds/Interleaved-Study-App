// Document import: files or folders; .md/.txt natively, PDF and DOCX best effort.
// Unchanged files (same content hash) are never re-analyzed; changed files keep history where topics still match.
// Reading and preparing files happens off the app's lock; only applying the result takes it.
use crate::core::Core;
use crate::preprocess::{prepare, Prepared};
use crate::rx;
use crate::types::*;
use crate::util::{course_code, natural_cmp, now_ms, sha256, uid};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const TEXT_EXT: [&str; 3] = ["md", "markdown", "txt"];
pub const ALL_EXT: [&str; 5] = ["md", "markdown", "txt", "pdf", "docx"];
/// What the file picker offers: lessons plus pictures (pictures only import when the AI can read them).
pub const PICK_EXT: [&str; 9] = ["md", "markdown", "txt", "pdf", "docx", "png", "jpg", "jpeg", "webp"];
const BEST_EFFORT_NOTE: &str = "PDF and DOCX import is best effort. Markdown (.md) gives the cleanest topics.";
const MAX_FILE_BYTES: u64 = 60 * 1024 * 1024;

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn walk(paths: &[String], max_depth: usize) -> Vec<PathBuf> {
    fn visit(p: &Path, depth: usize, max: usize, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
        let Ok(md) = std::fs::metadata(p) else { return };
        if md.is_dir() {
            if depth > max { return }
            let Ok(rd) = std::fs::read_dir(p) else { return };
            let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with('.') && n != "node_modules").collect();
            names.sort_by(|a, b| natural_cmp(a, b));
            for n in names { visit(&p.join(n), depth + 1, max, out, seen); }
        } else {
            // Pictures are taken only when chosen one by one, never swept up from a folder of course material.
            let ext = ext_of(p);
            let ok = ALL_EXT.contains(&ext.as_str()) || (depth == 0 && crate::files::is_image(&ext));
            if ok && seen.insert(p.to_path_buf()) { out.push(p.to_path_buf()); }
        }
    }
    let (mut out, mut seen) = (vec![], HashSet::new());
    for p in paths { visit(Path::new(p), 0, max_depth, &mut out, &mut seen); }
    out
}

fn decode_entities(s: &str) -> String {
    let s = s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'");
    let s = rx!(r"&#(x[0-9a-fA-F]+|\d+);").replace_all(&s, |c: &regex::Captures| {
        let n = if let Some(h) = c[1].strip_prefix('x') { u32::from_str_radix(h, 16).ok() } else { c[1].parse::<u32>().ok() };
        n.and_then(char::from_u32).map(|ch| ch.to_string()).unwrap_or_default()
    }).to_string();
    s.replace("&amp;", "&")
}

/// WordprocessingML → Markdown: headings from paragraph styles, list items as bullets, tables as rows of text.
pub fn docx_xml_to_markdown(xml: &str) -> String {
    let mut out: Vec<String> = vec![];
    for p in rx!(r"(?s)<w:p[ >].*?</w:p>|<w:p/>").find_iter(xml) {
        let p = p.as_str();
        let style = rx!(r#"<w:pStyle[^>]*w:val="([^"]+)""#).captures(p).map(|c| c[1].to_string()).unwrap_or_default();
        let mut text = String::new();
        for t in rx!(r"(?s)<w:t(?: [^>]*)?>(.*?)</w:t>|<w:tab/>|<w:br[^>]*/>|<w:cr/>").captures_iter(p) {
            match t.get(1) {
                Some(m) => text.push_str(&decode_entities(m.as_str())),
                None => text.push(if t[0].starts_with("<w:tab") { ' ' } else { '\n' }),
            }
        }
        let text = text.trim();
        if text.is_empty() { continue }
        let level = rx!(r"(?i)^heading\s*([1-6])$").captures(&style).and_then(|c| c[1].parse::<usize>().ok()).or(if style.eq_ignore_ascii_case("title") { Some(1) } else { None });
        out.push(match level {
            Some(l) => format!("{} {}", "#".repeat(l), text.replace('\n', " ")),
            None if p.contains("<w:numPr>") || style.to_lowercase().starts_with("list") => format!("- {text}"),
            None => text.to_string(),
        });
    }
    // Consecutive list items stay one block; everything else is its own paragraph.
    let mut md = String::new();
    for (i, l) in out.iter().enumerate() {
        if i > 0 { md.push_str(if l.starts_with("- ") && out[i - 1].starts_with("- ") { "\n" } else { "\n\n" }); }
        md.push_str(l);
    }
    md
}

fn read_docx(bytes: &[u8]) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| "This .docx file could not be opened (it may be damaged or password-protected).".to_string())?;
    let mut entry = zip.by_name("word/document.xml").map_err(|_| "This .docx file has no document text.".to_string())?;
    let mut xml = String::new();
    entry.by_ref().take(200 * 1024 * 1024).read_to_string(&mut xml).map_err(|e| e.to_string())?;
    Ok(docx_xml_to_markdown(&xml))
}

/// The PDF's text and page count. The page count comes from the same parse, so a hostile file is parsed once.
fn read_pdf(bytes: &[u8]) -> Result<(String, i64), String> {
    // The PDF parser can panic on malformed files; that must never take the app down.
    let res = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(bytes));
    let pages = match res {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(format!("This PDF could not be read ({e}).")),
        Err(_) => return Err("This PDF could not be read (its structure is not supported).".into()),
    };
    let n = pages.len() as i64;
    let text = pages.join("\n\n");
    // Re-flow: lines inside a paragraph are joined; blank lines separate paragraphs.
    Ok((rx!(r"\n\s*\n").split(&text).map(|p| rx!(r"\s*\n\s*").replace_all(p.trim(), " ").to_string()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n\n"), n))
}

/// The original file as read at import: what is needed to send it later and to know it has not changed.
#[derive(Clone, Debug)]
pub struct FileInfo {
    pub hash: String,
    pub bytes: i64,
    pub pages: Option<i64>,
    /// The content matches the type the extension claims.
    pub genuine: bool,
}

/// Read a file once, into a buffer no larger than the import limit. Only regular files: a device or a pipe named
/// like a lesson reports no size and would never end.
fn read_capped(file: &Path) -> Result<Vec<u8>, String> {
    let md = std::fs::metadata(file).map_err(|e| e.to_string())?;
    if !md.is_file() { return Err("This is not a regular file.".into()) }
    if md.len() > MAX_FILE_BYTES { return Err("This file is larger than 60 MB.".into()) }
    let mut buf = Vec::with_capacity(md.len() as usize);
    std::fs::File::open(file).and_then(|f| f.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf)).map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_FILE_BYTES { return Err("This file is larger than 60 MB.".into()) }
    Ok(buf)
}

/// The text the Mac can read from a file (empty for a picture or a scanned PDF), a best-effort note, and the file.
pub fn read_document(file: &Path) -> Result<(String, Option<&'static str>, FileInfo), String> {
    let ext = ext_of(file);
    if !PICK_EXT.contains(&ext.as_str()) { return Err(format!("Unsupported file type .{ext}. Use PDF, Word, Markdown, text or a picture.")) }
    let bytes = read_capped(file)?;
    let mut info = FileInfo { hash: crate::files::sha256_bytes(&bytes), bytes: bytes.len() as i64, pages: None, genuine: crate::files::magic_ok(&ext, &bytes) };
    let (text, note) = match ext.as_str() {
        e if TEXT_EXT.contains(&e) => (String::from_utf8_lossy(&bytes).to_string(), None),
        _ if !info.genuine => return Err(format!("This file is not really a .{ext} file.")),
        "pdf" => match read_pdf(&bytes) {
            Ok((t, n)) => { info.pages = Some(n); (t, Some(BEST_EFFORT_NOTE)) }
            // A PDF the Mac cannot parse may still be readable by the AI; the caller decides.
            Err(_) => (String::new(), Some(BEST_EFFORT_NOTE)),
        },
        "docx" => (read_docx(&bytes)?, Some(BEST_EFFORT_NOTE)),
        _ => (String::new(), None), // a picture: only the AI can read it
    };
    Ok((text, note, info))
}

pub struct ReadFile {
    pub path: PathBuf,
    pub result: Result<(Prepared, Option<&'static str>, FileInfo), String>,
}

/// Step 1, off the lock: find the files, read them and prepare their text.
pub fn read_all(paths: &[String]) -> Vec<ReadFile> {
    walk(paths, 6).into_iter().map(|path| {
        let result = read_document(&path).map(|(text, note, info)| (prepare(&text), note, info));
        ReadFile { path, result }
    }).collect()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
}

/// Step 2, under the lock: store what was read. With `course_id`, every new lesson goes into that course; without,
/// courses are inferred from the files. Lessons imported before stay in their course either way.
pub fn apply(c: &Core, paths: &[String], files: Vec<ReadFile>, confirm: bool, course_id: Option<&str>) -> Result<ImportResult, String> {
    let mut res = ImportResult::default();
    let target = match course_id.filter(|x| !x.is_empty()) {
        Some(id) => Some(c.store.get::<Course>("courses", id).ok_or("That course no longer exists.")?),
        None => None,
    };
    if files.is_empty() {
        res.errors.push(PathError { path: paths.join(", "), error: "No .md, .txt, .pdf, .docx or picture files found".into() });
        return Ok(res);
    }
    // A folder whose files share one course code ("CHM110 - …") becomes that course; otherwise the folder name is used.
    let mut codes: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    for f in &files {
        if let (Some(code), Some(dir)) = (course_code(&file_name(&f.path)), f.path.parent()) { codes.entry(dir.to_path_buf()).or_default().insert(code); }
    }
    let course_for = |file: &Path| -> String {
        if let Some(code) = course_code(&file_name(file)) { return code }
        let dir = file.parent().unwrap_or(Path::new(""));
        match codes.get(dir) {
            Some(set) if set.len() == 1 => set.iter().next().cloned().unwrap_or_default(),
            _ => file_name(dir),
        }
    };
    let mut best_effort = false;
    let ai = c.settings.ai.clone();
    let caps = c.caps(&ai.model);
    for f in files {
        let path = f.path.to_string_lossy().to_string();
        let (prep, note, info) = match f.result {
            Ok(x) => x,
            Err(error) => { res.errors.push(PathError { path, error }); continue }
        };
        let ext = ext_of(&f.path);
        let support = crate::files::support(&ai, &caps, &ext);
        let sent_as_file = support.kind == "file";
        if note.is_some() && !sent_as_file { best_effort = true; }
        // No text on the Mac (a scanned PDF, a picture): the AI reads the file first, if it can.
        let unread = prep.paras.is_empty() || prep.words < 5;
        if unread && !sent_as_file {
            let error = if crate::files::is_image(&ext) {
                format!("{} Pictures are read by Gemini or the Claude scheduled task.", support.detail)
            } else {
                format!("No readable text found. {} can read scanned PDFs; switch to it in Settings and import again.", if ai.provider == "gemini" || ai.is_routine() { "A model that accepts PDFs" } else { "Gemini or the Claude scheduled task" })
            };
            res.errors.push(PathError { path, error });
            continue;
        }
        // An unread file has no text yet: the file itself identifies it.
        let hash = if unread { format!("file:{}", info.hash) } else { sha256(&prep.text) };
        let docs = c.store.all::<Doc>("docs");
        // A scan or picture already imported (and perhaps already read by the AI, which changed its text hash) is the
        // same lesson when its file is the same, wherever it now lies.
        if unread && docs.iter().any(|d| d.file_hash.as_deref() == Some(info.hash.as_str())) { res.unchanged += 1; continue }
        let existing = docs.iter().find(|d| d.path == path).cloned();
        let stored = DocPrep { paras: prep.paras.clone(), outline: prep.outline.clone() };
        match &existing {
            None => if docs.iter().any(|d| d.hash == hash) { res.unchanged += 1; continue },
            Some(e) if e.hash == hash => {
                // Older records may lack stored paragraphs: store them now without touching topics.
                if c.doc_prep(&e.id).is_none() { c.save_doc_prep(&e.id, &stored); }
                // Lessons imported before files could be sent learn their file now, from this fresh import.
                if e.file_hash.as_deref() != Some(info.hash.as_str()) {
                    let mut d = e.clone();
                    d.file_hash = Some(info.hash.clone());
                    d.file_bytes = Some(info.bytes);
                    d.pages = info.pages;
                    if sent_as_file && d.import_note.as_deref() == Some(BEST_EFFORT_NOTE) { d.import_note = None; }
                    c.store.put("docs", &d);
                }
                res.unchanged += 1;
                continue;
            }
            Some(_) => {}
        }
        let now = now_ms();
        if let Some(e) = existing {
            let with_history = c.history_count(&e.id);
            if with_history > 0 && !confirm {
                res.needs_confirm.push(ImportChange { path, doc_id: e.id.clone(), title: e.title.clone(), with_history });
                continue;
            }
            c.store.tx(|| {
                for j in c.store.by_parent::<Job>("jobs", &e.id) { if j.status != "running" { c.store.del("jobs", &j.id); } }
                for w in c.store.by_parent::<serde_json::Value>("wins", &e.id) { if let Some(id) = w["id"].as_str() { c.store.del("wins", id); } }
                let mut doc = e.clone();
                doc.hash = hash.clone();
                doc.words = prep.words;
                doc.para_count = prep.paras.len() as i64;
                doc.status = "awaiting".into();
                doc.windows = 0;
                doc.wins_done = 0;
                doc.error = None;
                doc.import_note = if sent_as_file { None } else { note.map(|n| n.to_string()) };
                doc.file_hash = Some(info.hash.clone());
                doc.file_bytes = Some(info.bytes);
                doc.pages = info.pages;
                doc.needs_reading = if unread { Some(true) } else { None };
                doc.read_by_ai = None;
                doc.updated_at = now;
                c.store.put("docs", &doc);
                c.save_doc_prep(&doc.id, &stored);
                let (_, dropped) = c.replace_doc_topics(&doc.id, c.provisional_items(&doc, &stored));
                if dropped > 0 {
                    doc.import_note = Some(format!("{dropped} earlier topic(s) with history had no match in the new version."));
                    c.store.put("docs", &doc);
                }
            });
            res.replaced += 1;
            res.doc_ids.push(e.id);
            continue;
        }
        let course = match &target { Some(t) => t.clone(), None => c.ensure_course(&course_for(&f.path)) };
        let name = file_name(&f.path);
        let stem = f.path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let doc = Doc {
            id: uid(), course_id: course.id.clone(), title: rx!(r"_+").replace_all(&stem, " ").trim().to_string(), title_locked: None, file_name: name, path,
            ext: ext_of(&f.path), hash, words: prep.words, para_count: prep.paras.len() as i64, windows: 0, wins_done: 0, status: "awaiting".into(),
            error: None, import_note: if sent_as_file { None } else { note.map(|n| n.to_string()) }, created_at: now, updated_at: now,
            file_hash: Some(info.hash.clone()), file_bytes: Some(info.bytes), pages: info.pages,
            needs_reading: if unread { Some(true) } else { None }, read_by_ai: None,
        };
        c.store.tx(|| {
            c.store.put("docs", &doc);
            c.save_doc_prep(&doc.id, &stored);
            c.replace_doc_topics(&doc.id, c.provisional_items(&doc, &stored));
        });
        res.added += 1;
        res.doc_ids.push(doc.id);
    }
    if best_effort { res.notes.push(BEST_EFFORT_NOTE.into()); }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Fx;
    use crate::store::Store;

    #[test]
    fn docx_paragraphs_headings_and_lists() {
        let xml = r#"<w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Cells &amp; energy</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">First </w:t></w:r><w:r><w:t>sentence.</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>one</w:t></w:r></w:p>
<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>two</w:t></w:r></w:p></w:body>"#;
        assert_eq!(docx_xml_to_markdown(xml), "# Cells & energy\n\nFirst sentence.\n\n- one\n- two");
    }

    #[test]
    fn a_read_picture_reimports_as_unchanged() {
        let dir = std::env::temp_dir().join(format!("is-import-pic-{}", uid()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Board.png");
        std::fs::write(&file, b"\x89PNG\r\n\x1a\n fake picture body").unwrap();
        let copy = dir.join("Board copy.png");
        std::fs::copy(&file, &copy).unwrap();
        let c = Core::new(Store::memory(), Fx::default());
        let course = c.create_course("BIO").unwrap().id;
        let paths = vec![file.to_string_lossy().to_string()];
        let r = apply(&c, &paths, read_all(&paths), false, Some(&course)).unwrap();
        assert_eq!((r.added, r.errors.len()), (1, 0));
        // The AI has since written out its text, which changes the lesson's text hash.
        let mut d = c.store.all::<Doc>("docs").remove(0);
        assert_eq!(d.needs_reading, Some(true));
        d.hash = "text-hash".into();
        d.needs_reading = None;
        d.read_by_ai = Some(true);
        c.store.put("docs", &d);
        let again = vec![file.to_string_lossy().to_string(), copy.to_string_lossy().to_string()];
        let r2 = apply(&c, &again, read_all(&again), false, Some(&course)).unwrap();
        assert_eq!((r2.added, r2.replaced, r2.unchanged), (0, 0, 2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_adds_then_skips_unchanged() {
        let dir = std::env::temp_dir().join(format!("is-import-{}", uid()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("BIO101 - Cells.md");
        std::fs::write(&file, "# Cells\n\nCells are the basic unit of life and every organism is made of them.\n\n## Membranes\n\nThe membrane controls what enters and leaves the cell at all times.\n").unwrap();
        let c = Core::new(Store::memory(), Fx::default());
        let paths = vec![dir.to_string_lossy().to_string()];
        let r = apply(&c, &paths, read_all(&paths), false, None).unwrap();
        assert_eq!((r.added, r.errors.len()), (1, 0));
        assert_eq!(c.store.all::<Course>("courses")[0].name, "BIO101");
        assert!(!c.store.all::<Topic>("topics").is_empty());
        let r2 = apply(&c, &paths, read_all(&paths), false, None).unwrap();
        assert_eq!((r2.added, r2.unchanged), (0, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
