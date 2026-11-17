// Which lesson files go to the AI as the original file, and the attachment itself.
// Only Google Gemini (API key) and the Claude scheduled task receive files; Claude API and OpenAI get the text the
// Mac extracted, as before. A PDF sent as a file is read by the model the way chat apps do: its text plus an image
// of every page. The file is read from disk at request time, checked against the hash taken at import, and never
// stored in the database or a job.
use crate::settings::AiSettings;
use crate::types::{Doc, ModelCaps};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

/// Largest file sent inline to Gemini (base64 makes it about a third larger on the wire).
pub const GEMINI_MAX_BYTES: i64 = 15 * 1024 * 1024;
/// Largest file put in the scheduled task's mailbox, and the most PDF pages its session is asked to read.
pub const ROUTINE_MAX_BYTES: i64 = 20 * 1024 * 1024;
pub const ROUTINE_MAX_PAGES: i64 = 100;
/// Gemini bills each PDF page as about 258 tokens and an image as about 1,100.
pub const PAGE_TOKENS: f64 = 258.0;
pub const IMAGE_TOKENS: f64 = 1100.0;
/// Most pages the AI is asked to write out in one request: longer scans would outrun the request's time limit.
pub const READ_MAX_PAGES: i64 = 40;

pub const IMAGE_EXT: [&str; 4] = ["png", "jpg", "jpeg", "webp"];
/// Every extension the support table describes, in the order Settings shows them.
pub const TABLE_EXT: [&str; 8] = ["pdf", "docx", "md", "txt", "png", "jpg", "jpeg", "webp"];

pub fn is_image(ext: &str) -> bool { IMAGE_EXT.contains(&ext) }

pub fn mime(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// How one file type reaches the current model.
/// kind: "file" (the original file is sent), "text" (sent as written), "extracted" (the Mac pulls out the text,
/// best effort), "unsupported" (cannot be imported with this provider).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FileSupport {
    pub ext: String,
    pub kind: &'static str,
    pub label: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_pages: Option<i64>,
}

pub fn provider_name(ai: &AiSettings) -> &'static str {
    match ai.provider.as_str() { "gemini" => "Gemini", "anthropic" => "Claude API", "openai" => "OpenAI", "routine" => "the Claude scheduled task", _ => "this provider" }
}

/// Providers that receive original files at all. Gemini only at Google's own address (or this Mac, for tests): a
/// custom address was confirmed for lesson text, not for whole files.
pub fn sends_files(ai: &AiSettings) -> bool {
    ai.is_routine() || (ai.provider == "gemini" && gemini_host_ok(ai))
}

pub fn gemini_host_ok(ai: &AiSettings) -> bool {
    let base = ai.base_url("gemini");
    base.trim_end_matches('/') == crate::settings::default_base_url("gemini").trim_end_matches('/')
        || url::Url::parse(&base).is_ok_and(|u| matches!(u.host_str(), Some("localhost") | Some("127.0.0.1") | Some("[::1]")))
}

/// A Gemini model's refusal of a type; the scheduled task never inherits it from the Gemini model left in Settings.
fn rejected(ai: &AiSettings, caps: &ModelCaps, mime: &str) -> bool {
    ai.provider == "gemini" && caps.file_rejected.as_ref().is_some_and(|l| l.iter().any(|m| m == mime))
}

pub fn support(ai: &AiSettings, caps: &ModelCaps, ext: &str) -> FileSupport {
    let ext = ext.to_lowercase();
    let who = provider_name(ai);
    let routine = ai.is_routine();
    let (max_bytes, max_pages) = if routine { (Some(ROUTINE_MAX_BYTES), if ext == "pdf" { Some(ROUTINE_MAX_PAGES) } else { None }) } else { (Some(GEMINI_MAX_BYTES), None) };
    let mk = |kind: &'static str, label: &str, detail: String, limits: bool| FileSupport {
        ext: ext.clone(), kind, label: label.into(), detail, max_bytes: if limits { max_bytes } else { None }, max_pages: if limits { max_pages } else { None },
    };
    let extracted = |why: String| mk("extracted", "Text extracted", why, false);
    match ext.as_str() {
        "md" | "markdown" | "txt" => mk("text", "Sent as text", "Plain text reaches every model exactly as written.".into(), false),
        "pdf" | "docx" | "png" | "jpg" | "jpeg" | "webp" => {
            let m = mime(&ext).unwrap_or_default();
            let image = is_image(&ext);
            // The scheduled task's session reads PDFs and pictures, not Word files.
            let can = sends_files(ai) && !(routine && ext == "docx");
            if can && !rejected(ai, caps, m) {
                let detail = match ext.as_str() {
                    "pdf" => "The model reads the text and an image of every page, so tables, equations and figures count. Scanned PDFs work too.".to_string(),
                    "docx" => "The model reads the Word file itself, including its tables.".to_string(),
                    _ => "The model reads the picture and writes out its text, so photos of notes or slides become lessons.".to_string(),
                };
                return mk("file", if image { "Sent as image" } else if ext == "pdf" { "Sent as PDF" } else { "Sent as file" }, detail, true);
            }
            let custom = ai.provider == "gemini" && !gemini_host_ok(ai);
            if custom {
                let why = "Files only go to Google's own Gemini address. With a custom address the Mac extracts the text instead.".to_string();
                return if image { mk("unsupported", "Not supported", why, false) } else { extracted(why) };
            }
            if image {
                let why = if can { format!("{} turned pictures down. Choose another model in Settings.", ai.model) } else { format!("Pictures can only be read by Gemini or the Claude scheduled task, not {who}.") };
                return mk("unsupported", "Not supported", why, false);
            }
            if can { return extracted(format!("{} turned this file type down, so the Mac extracts the text instead (best effort).", ai.model)) }
            extracted(if ext == "docx" && routine {
                "The scheduled task cannot open Word files, so the Mac extracts the text (best effort). Save as PDF to send the file itself.".into()
            } else {
                format!("{who} gets the text the Mac extracts (best effort). Gemini and the Claude scheduled task can read the file itself.")
            })
        }
        _ => mk("unsupported", "Not supported", "Use PDF, Word, Markdown, text or a picture.".into(), false),
    }
}

pub fn support_table(ai: &AiSettings, caps: &ModelCaps) -> Vec<FileSupport> {
    TABLE_EXT.iter().filter(|e| **e != "jpeg").map(|e| support(ai, caps, e)).collect()
}

/// Whether this lesson goes to the model with its original file: the type is sent as a file, the import recorded
/// the file's hash, it is within the size and page limits, and its text did not already come from the AI reading it.
pub fn plan(ai: &AiSettings, caps: &ModelCaps, doc: &Doc) -> Option<&'static str> {
    let s = support(ai, caps, &doc.ext);
    if s.kind != "file" || doc.file_hash.is_none() { return None }
    if doc.read_by_ai == Some(true) && doc.needs_reading != Some(true) { return None }
    if s.max_bytes.is_some_and(|m| doc.file_bytes.unwrap_or(i64::MAX) > m) { return None }
    if s.max_pages.is_some_and(|m| doc.pages.unwrap_or(0) > m) { return None }
    mime(&doc.ext)
}

/// Why a lesson that could go as a file does not (too large, too many pages), for the lesson's note.
pub fn plan_problem(ai: &AiSettings, caps: &ModelCaps, doc: &Doc) -> Option<String> {
    let s = support(ai, caps, &doc.ext);
    if s.kind != "file" { return None }
    let mb = |b: i64| (b as f64 / 1_048_576.0).round() as i64;
    if let (Some(m), Some(b)) = (s.max_bytes, doc.file_bytes) { if b > m { return Some(format!("This file is {} MB, over the {} MB that can be sent as a file, so the AI gets the extracted text.", mb(b), mb(m))) } }
    if let (Some(m), Some(p)) = (s.max_pages, doc.pages) { if p > m { return Some(format!("This PDF has {p} pages, over the {m} the scheduled task is asked to read, so it gets the extracted text.")) } }
    None
}

/// Extra input tokens one request pays for carrying this lesson's file.
pub fn file_tokens(doc: &Doc) -> f64 {
    match doc.ext.as_str() {
        "pdf" => doc.pages.unwrap_or(1).max(1) as f64 * PAGE_TOKENS,
        e if is_image(e) => IMAGE_TOKENS,
        _ => doc.words as f64 * 1.4,
    }
}

#[derive(Clone, Debug)]
pub struct Attachment {
    /// A name made from the content hash; the person's file name never leaves the Mac this way.
    pub name: String,
    pub mime: String,
    pub hash: String,
    pub pages: Option<i64>,
    pub data: Arc<Vec<u8>>,
}

fn magic_ok_name(ext: &str) -> bool { mime(ext).is_some() }

pub fn sha256_bytes(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// Read a lesson's original file for sending: same size limit, same content as at import. Call it off the lock.
pub fn load(path: &str, ext: &str, mime: &str, expect_hash: &str, max_bytes: i64, pages: Option<i64>) -> Result<Attachment, String> {
    let p = Path::new(path);
    if !magic_ok_name(ext) { return Err("This file type is not sent as a file.".into()) }
    // A symlink swapped in after import is not followed.
    let meta = std::fs::symlink_metadata(p).map_err(|_| "The original file is no longer where it was imported from.".to_string())?;
    if !meta.is_file() { return Err("The original file is no longer a regular file.".into()) }
    if meta.len() as i64 > max_bytes { return Err("The original file grew past the size that can be sent.".into()) }
    let mut data = Vec::with_capacity(meta.len() as usize);
    std::fs::File::open(p).and_then(|f| f.take(max_bytes as u64 + 1).read_to_end(&mut data)).map_err(|e| format!("The original file could not be read ({e})."))?;
    if data.len() as i64 > max_bytes { return Err("The original file grew past the size that can be sent.".into()) }
    let hash = sha256_bytes(&data);
    if hash != expect_hash { return Err("The original file changed since it was imported. Import it again to send the new version.".into()) }
    if !magic_ok(ext, &data) { return Err("The original file is not the type its name says.".into()) }
    let ext = if ext == "jpeg" { "jpg" } else { ext };
    Ok(Attachment { name: format!("{}.{ext}", &hash[..16]), mime: mime.into(), hash, pages, data: Arc::new(data) })
}

/// Whether the bytes really are the type the extension claims, so a renamed file is never sent as something else.
pub fn magic_ok(ext: &str, b: &[u8]) -> bool {
    match ext {
        // The header may follow a little junk, as readers allow.
        "pdf" => b[..b.len().min(1024)].windows(5).any(|w| w == b"%PDF-"),
        "docx" => b.starts_with(b"PK\x03\x04"),
        "png" => b.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => b.starts_with(&[0xFF, 0xD8, 0xFF]),
        "webp" => b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP",
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ai(provider: &str) -> AiSettings {
        let mut s = crate::settings::typed(&crate::settings::default_settings()).ai;
        s.provider = provider.into();
        s
    }

    #[test]
    fn only_gemini_and_routine_get_files() {
        let caps = ModelCaps::default();
        assert_eq!(support(&ai("gemini"), &caps, "pdf").kind, "file");
        assert_eq!(support(&ai("routine"), &caps, "pdf").kind, "file");
        assert_eq!(support(&ai("anthropic"), &caps, "pdf").kind, "extracted");
        assert_eq!(support(&ai("openai"), &caps, "pdf").kind, "extracted");
        assert_eq!(support(&ai("routine"), &caps, "docx").kind, "extracted");
        assert_eq!(support(&ai("gemini"), &caps, "png").kind, "file");
        assert_eq!(support(&ai("openai"), &caps, "png").kind, "unsupported");
        assert_eq!(support(&ai("anthropic"), &caps, "md").kind, "text");
    }

    #[test]
    fn rejected_type_falls_back() {
        let caps = ModelCaps { file_rejected: Some(vec!["application/pdf".into()]), ..Default::default() };
        assert_eq!(support(&ai("gemini"), &caps, "pdf").kind, "extracted");
        assert_eq!(support(&ai("routine"), &caps, "pdf").kind, "file");
    }

    #[test]
    fn plan_respects_limits() {
        let caps = ModelCaps::default();
        let mut d = Doc { ext: "pdf".into(), file_hash: Some("h".into()), file_bytes: Some(1000), pages: Some(150), ..Default::default() };
        assert!(plan(&ai("gemini"), &caps, &d).is_some());
        assert!(plan(&ai("routine"), &caps, &d).is_none());
        d.pages = Some(20);
        assert!(plan(&ai("routine"), &caps, &d).is_some());
        d.file_bytes = Some(GEMINI_MAX_BYTES + 1);
        assert!(plan(&ai("gemini"), &caps, &d).is_none());
        d.file_hash = None;
        d.file_bytes = Some(10);
        assert!(plan(&ai("gemini"), &caps, &d).is_none());
    }

    #[test]
    fn load_checks_hash() {
        let dir = std::env::temp_dir().join(format!("is-files-{}", crate::util::uid()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.pdf");
        std::fs::write(&f, b"%PDF-1.4 test").unwrap();
        let path = f.to_string_lossy().to_string();
        let h = sha256_bytes(b"%PDF-1.4 test");
        assert!(magic_ok("pdf", b"%PDF-1.4") && !magic_ok("pdf", b"<html>") && !magic_ok("png", b"%PDF-"));
        let a = load(&path, "pdf", "application/pdf", &h, 1000, None).unwrap();
        assert_eq!(a.name, format!("{}.pdf", &h[..16]));
        assert!(load(&path, "pdf", "application/pdf", "other", 1000, None).is_err());
        assert!(load(&path, "pdf", "application/pdf", &h, 3, None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
