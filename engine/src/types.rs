// Records as they are stored (JSON in SQLite) and sent to the window. Field names match the TypeScript types.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Rating { Lost, Shaky, Okay, Solid }

impl Rating {
    pub fn label(self) -> &'static str {
        match self { Rating::Lost => "lost", Rating::Shaky => "shaky", Rating::Okay => "okay", Rating::Solid => "solid" }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub at: f64,
    pub rating: Rating,
    #[serde(default, rename = "self")]
    pub self_rating: Option<Rating>,
    #[serde(default)]
    pub minutes: f64,
    #[serde(default)]
    pub quiz: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Course {
    pub id: String,
    pub name: String,
    pub color: i64,
    pub created_at: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Doc {
    pub id: String,
    pub course_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_locked: Option<bool>,
    pub file_name: String,
    pub path: String,
    pub ext: String,
    pub hash: String,
    pub words: i64,
    pub para_count: i64,
    pub windows: i64,
    pub wins_done: i64,
    pub status: String, // awaiting | queued | organized | needs | error
    pub error: Option<String>,
    pub import_note: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    /// sha256 of the original file's bytes at import (`hash` is of the prepared text).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<i64>,
    /// No text could be read on the Mac (a scanned PDF, a picture): the AI reads the file first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_reading: Option<bool>,
    /// The lesson's text was written out by the AI from the file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_by_ai: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Topic {
    pub id: String,
    pub doc_id: String,
    pub course_id: String,
    pub unit: String,
    pub name: String,
    pub ord: f64,
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub words: i64,
    pub minutes: f64,
    pub summary: String,
    pub terms: Vec<String>,
    pub prereqs: Vec<String>,
    pub difficulty: Option<f64>,
    pub provisional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<bool>,
    pub notes: String,
    pub created_at: f64,
    // schedule
    pub stab: Option<f64>,
    pub due: Option<f64>,
    pub last: Option<f64>,
    pub evid: Option<f64>,
    pub reviews: Vec<Review>,
    pub spent: f64,
    #[serde(rename = "override")]
    pub override_score: Option<f64>,
    pub solid_mark: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Heading {
    pub level: i64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Para {
    pub id: i64,
    pub text: String,
    #[serde(default)]
    pub words: i64,
    #[serde(default)]
    pub heading: Option<Heading>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DocPrep {
    #[serde(default)]
    pub paras: Vec<Para>,
    #[serde(default)]
    pub outline: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Deadline {
    pub id: String,
    pub title: String,
    pub date: String,
    pub course_id: Option<String>,
    pub doc_id: Option<String>,
    pub topic_ids: Vec<String>,
    pub notes: String,
    pub key: String,
    pub created_at: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DayGoal {
    pub kind: String,
    pub value: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Daily {
    pub id: String,
    pub minutes: f64,
    pub topics: f64,
    pub goal: Option<DayGoal>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Snapshot {
    pub id: String,
    pub avg: f64,
    pub solid: i64,
    pub progress: i64,
    pub review: i64,
    pub fresh: i64,
    pub total: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Job {
    pub id: String,
    pub kind: String, // organize | dedupe | enrich | quiz | grade | deadlines
    pub doc_id: Option<String>,
    pub topic_id: Option<String>,
    pub payload: Value,
    pub status: String, // queued | running | done | failed
    pub attempts: i64,
    pub next_at: f64,
    pub error: Option<String>,
    pub est_tokens: f64,
    pub priority: i64,
    pub created_at: f64,
    pub label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QuizQuestion {
    #[serde(rename = "type")]
    pub kind: String, // mc | short
    pub q: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    pub answer: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Quiz {
    pub id: String,
    pub text_hash: String,
    pub questions: Vec<QuizQuestion>,
    pub created_at: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Grade {
    pub correct: bool,
    pub feedback: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Pool {
    Topic { ids: Vec<String> },
    Sandbox { ids: Vec<String> },
    #[serde(rename_all = "camelCase")]
    Unit { doc_id: String, unit: String },
    #[serde(rename_all = "camelCase")]
    Lesson { doc_id: String },
    #[serde(rename_all = "camelCase")]
    Course { course_ids: Vec<String> },
    Global,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub topic_id: String,
    pub name: String,
    pub score: f64,
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RibbonBlock {
    pub topic_id: String,
    pub name: String,
    pub course_id: String,
    pub state: String, // rated | skipped | current
    #[serde(default)]
    pub rating: Option<Rating>,
    #[serde(default)]
    pub minutes: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PendingReview {
    pub topic_id: String,
    pub rating: Option<Rating>,
    #[serde(default)]
    pub quiz: Option<f64>,
    pub minutes: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub at: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub active: bool,
    pub id: String,
    pub mode: String, // timed | pomodoro
    pub pool: Pool,
    pub pool_label: String,
    pub sandbox: bool,
    pub phase: String, // work | break | rate | next | done
    pub break_kind: Option<String>,
    pub topic_id: Option<String>,
    pub topic_name: String,
    pub course_id: Option<String>,
    pub end_at: f64,
    pub paused_at: Option<f64>,
    pub duration_ms: f64,
    pub work_ms: f64,
    pub block_index: usize,
    pub blocks_for_topic: usize,
    pub block_plan: Vec<f64>,
    pub cycles: i64,
    pub blocks: Vec<RibbonBlock>,
    pub proposal: Option<Suggestion>,
    #[serde(default)]
    pub passed: Vec<String>,
    pub last_topic_id: Option<String>,
    pub last_doc_id: Option<String>,
    pub last_was_review: bool,
    #[serde(default)]
    pub sandbox_reviews: Vec<PendingReview>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitInfo {
    pub at: f64,
    pub reset_at: Option<f64>,
    pub message: String,
    pub provider: String,
    pub model: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FailedJob {
    pub id: String,
    pub label: String,
    pub error: String,
    pub doc_id: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QueueStatus {
    pub user_paused: bool,
    pub paused_until: Option<f64>,
    pub pause_reason: Option<String>,
    pub running: Option<String>,
    pub running_id: Option<String>,
    pub queued: usize,
    pub failed: Vec<FailedJob>,
    pub docs_total: usize,
    pub docs_done: usize,
    pub chunks_total: usize,
    pub chunks_done: usize,
    pub rate_limit: Option<RateLimitInfo>,
    pub no_key: bool,
    /// A Claude scheduled-task run that is being waited on, when there is one.
    pub run: Option<RunInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub batch: String,
    pub requests: usize,
    pub fired_at: f64,
    pub url: Option<String>,
    pub state: String, // waiting | sending | holding
    pub next_fire_at: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelCaps {
    pub model: String,
    pub api: String, // generateContent | interactions | messages | responses | routine
    #[serde(default)]
    pub system: Option<bool>,
    #[serde(default)]
    pub json: Option<bool>,
    #[serde(default)]
    pub think_minimal: Option<bool>,
    #[serde(default)]
    pub think_level: Option<String>,
    #[serde(default)]
    pub fallback: Option<bool>,
    #[serde(default)]
    pub schema: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_fails: Option<i64>,
    #[serde(default)]
    pub room: Option<bool>,
    /// File types (MIME) this model turned down when sent as a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_rejected: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tested_at: Option<f64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub ok: bool,
    pub model: String,
    pub api: String,
    pub latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub caps: ModelCaps,
    pub notes: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedDeadline {
    pub title: String,
    pub date: String,
    pub course_name: Option<String>,
    pub notes: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DeadlineInput {
    pub title: String,
    pub date: String,
    pub course_id: Option<String>,
    pub doc_id: Option<String>,
    pub topic_ids: Option<Vec<String>>,
    pub notes: Option<String>,
    pub course_name: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportChange {
    pub path: String,
    pub doc_id: String,
    pub title: String,
    pub with_history: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct PathError {
    pub path: String,
    pub error: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub added: usize,
    pub unchanged: usize,
    pub replaced: usize,
    pub needs_confirm: Vec<ImportChange>,
    pub errors: Vec<PathError>,
    pub notes: Vec<String>,
    pub doc_ids: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Estimate {
    pub docs: usize,
    pub requests: usize,
    pub cached: usize,
    pub tokens: f64,
}
