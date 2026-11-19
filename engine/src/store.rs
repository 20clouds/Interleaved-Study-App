// SQLite store (WAL, FTS5). Each collection is a table (id, parent, data JSON); raw text lives in its own table so
// list queries never load it. The layout is the same one the Electron version used, so its database opens as is.
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::cell::Cell;
use std::path::Path;

/// State about this Mac's scheduled-task mailbox (batches in flight, answers kept for later, recent fires). It acts
/// on GitHub with this Mac's sign-in, so it is neither written into a backup nor taken from one.
pub const MACHINE_KV: [&str; 3] = ["routinePending", "routineResults", "routineFires"];

pub const COLLS: [&str; 10] = ["courses", "docs", "topics", "jobs", "deadlines", "daily", "snapshots", "quizzes", "cache", "wins"];

fn parent_field(c: &str) -> Option<&'static str> {
    match c {
        "topics" | "jobs" | "wins" => Some("docId"),
        "docs" => Some("courseId"),
        _ => None,
    }
}

fn coll(c: &str) -> &'static str {
    COLLS.iter().find(|x| **x == c).copied().expect("known collection")
}

pub struct Store {
    conn: Connection,
    depth: Cell<u32>,
}

impl Store {
    pub fn open(file: &Path) -> rusqlite::Result<Store> {
        Self::init(Connection::open(file)?)
    }

    pub fn memory() -> Store {
        Self::init(Connection::open_in_memory().expect("in-memory db")).expect("schema")
    }

    fn init(conn: Connection) -> rusqlite::Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        for c in COLLS {
            conn.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS {c} (id TEXT PRIMARY KEY, parent TEXT, data TEXT NOT NULL);
                 CREATE INDEX IF NOT EXISTS {c}_parent ON {c}(parent);"
            ))?;
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS texts (id TEXT PRIMARY KEY, text TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS kv (k TEXT PRIMARY KEY, v TEXT NOT NULL);
             CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(id UNINDEXED, name, body, tokenize = 'porter unicode61');",
        )?;
        conn.set_prepared_statement_cache_capacity(96);
        Ok(Store { conn, depth: Cell::new(0) })
    }

    fn rows(&self, sql: &str, p: impl rusqlite::Params) -> Vec<String> {
        let Ok(mut st) = self.conn.prepare_cached(sql) else { return vec![] };
        let it = st.query_map(p, |r| r.get::<_, String>(0));
        match it {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => vec![],
        }
    }

    pub fn get<T: DeserializeOwned>(&self, c: &str, id: &str) -> Option<T> {
        let sql = format!("SELECT data FROM {} WHERE id = ?1", coll(c));
        let s: Option<String> = self.conn.prepare_cached(&sql).ok()?.query_row(params![id], |r| r.get(0)).optional().ok()?;
        serde_json::from_str(&s?).ok()
    }

    pub fn has(&self, c: &str, id: &str) -> bool {
        let sql = format!("SELECT 1 FROM {} WHERE id = ?1", coll(c));
        self.conn.prepare_cached(&sql).ok().and_then(|mut s| s.query_row(params![id], |_| Ok(())).optional().ok().flatten()).is_some()
    }

    /// Every record of a collection. Records that no longer fit their shape are skipped, never fatal.
    pub fn all<T: DeserializeOwned>(&self, c: &str) -> Vec<T> {
        self.rows(&format!("SELECT data FROM {}", coll(c)), []).iter().filter_map(|s| serde_json::from_str(s).ok()).collect()
    }

    pub fn by_parent<T: DeserializeOwned>(&self, c: &str, parent: &str) -> Vec<T> {
        self.rows(&format!("SELECT data FROM {} WHERE parent = ?1", coll(c)), params![parent]).iter().filter_map(|s| serde_json::from_str(s).ok()).collect()
    }

    pub fn count_parent(&self, c: &str, parent: &str) -> usize {
        let sql = format!("SELECT COUNT(*) FROM {} WHERE parent = ?1", coll(c));
        self.conn.prepare_cached(&sql).ok().and_then(|mut s| s.query_row(params![parent], |r| r.get::<_, i64>(0)).ok()).unwrap_or(0) as usize
    }

    pub fn put<T: Serialize>(&self, c: &str, obj: &T) {
        let Ok(v) = serde_json::to_value(obj) else { return };
        self.put_value(c, &v);
    }

    pub fn put_value(&self, c: &str, v: &Value) {
        let Some(id) = v.get("id").and_then(|x| x.as_str()) else { return };
        let parent = parent_field(c).and_then(|f| v.get(f)).and_then(|x| x.as_str());
        let sql = format!(
            "INSERT INTO {} (id, parent, data) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET parent = excluded.parent, data = excluded.data",
            coll(c)
        );
        if let Ok(mut st) = self.conn.prepare_cached(&sql) {
            let _ = st.execute(params![id, parent, v.to_string()]);
        }
    }

    pub fn del(&self, c: &str, id: &str) {
        let _ = self.conn.execute(&format!("DELETE FROM {} WHERE id = ?1", coll(c)), params![id]);
    }

    pub fn get_text(&self, id: &str) -> Option<String> {
        self.conn.prepare_cached("SELECT text FROM texts WHERE id = ?1").ok()?.query_row(params![id], |r| r.get(0)).optional().ok()?
    }

    pub fn put_text(&self, id: &str, text: &str) {
        let _ = self.conn.execute("INSERT INTO texts (id, text) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET text = excluded.text", params![id, text]);
    }

    pub fn del_text(&self, id: &str) {
        let _ = self.conn.execute("DELETE FROM texts WHERE id = ?1", params![id]);
    }

    pub fn kv_get<T: DeserializeOwned>(&self, k: &str) -> Option<T> {
        let s: Option<String> = self.conn.prepare_cached("SELECT v FROM kv WHERE k = ?1").ok()?.query_row(params![k], |r| r.get(0)).optional().ok()?;
        serde_json::from_str(&s?).ok()
    }

    pub fn kv_set<T: Serialize>(&self, k: &str, v: &T) {
        match serde_json::to_string(v) {
            Ok(s) if s != "null" => {
                let _ = self.conn.execute("INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v", params![k, s]);
            }
            _ => self.kv_del(k),
        }
    }

    pub fn kv_del(&self, k: &str) {
        let _ = self.conn.execute("DELETE FROM kv WHERE k = ?1", params![k]);
    }

    pub fn index(&self, id: &str, name: &str, body: &str) {
        let _ = self.conn.execute("DELETE FROM fts WHERE id = ?1", params![id]);
        let _ = self.conn.execute("INSERT INTO fts (id, name, body) VALUES (?1, ?2, ?3)", params![id, name, body]);
    }

    pub fn unindex(&self, id: &str) {
        let _ = self.conn.execute("DELETE FROM fts WHERE id = ?1", params![id]);
    }

    pub fn search(&self, q: &str, limit: usize) -> Vec<String> {
        let lower = q.to_lowercase();
        let toks: Vec<String> = lower.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).take(12).map(|t| format!("\"{t}\"*")).collect();
        if toks.is_empty() { return vec![] }
        self.rows("SELECT id FROM fts WHERE fts MATCH ?1 ORDER BY rank LIMIT ?2", params![toks.join(" "), limit as i64])
    }

    /// Run `f` in one transaction (nested calls join the outer one).
    pub fn tx<R>(&self, f: impl FnOnce() -> R) -> R {
        if self.depth.get() > 0 { return f() }
        let began = self.conn.execute_batch("BEGIN").is_ok();
        self.depth.set(1);
        let r = f();
        self.depth.set(0);
        if began { let _ = self.conn.execute_batch("COMMIT"); }
        r
    }

    pub fn dump(&self) -> Value {
        let mut colls = Map::new();
        for c in COLLS { colls.insert(c.into(), Value::Array(self.all::<Value>(c))); }
        let mut texts = Map::new();
        if let Ok(mut st) = self.conn.prepare("SELECT id, text FROM texts") {
            if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                for (id, t) in rows.flatten() { texts.insert(id, Value::String(t)); }
            }
        }
        let mut kv = Map::new();
        if let Ok(mut st) = self.conn.prepare("SELECT k, v FROM kv") {
            if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                for (k, v) in rows.flatten().filter(|(k, _)| !MACHINE_KV.contains(&k.as_str())) { kv.insert(k, serde_json::from_str(&v).unwrap_or(Value::Null)); }
            }
        }
        json!({ "app": "interleaved-study", "version": 1, "exportedAt": crate::util::now_ms(), "colls": colls, "texts": texts, "kv": kv })
    }

    pub fn restore(&self, b: &Value) {
        self.tx(|| {
            for c in COLLS {
                let _ = self.conn.execute_batch(&format!("DELETE FROM {c}"));
                if let Some(list) = b.get("colls").and_then(|x| x.get(c)).and_then(|x| x.as_array()) {
                    for o in list { self.put_value(c, o); }
                }
            }
            let _ = self.conn.execute_batch("DELETE FROM texts; DELETE FROM fts;");
            let keep = MACHINE_KV.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
            let _ = self.conn.execute_batch(&format!("DELETE FROM kv WHERE k NOT IN ({keep})"));
            if let Some(m) = b.get("texts").and_then(|x| x.as_object()) {
                for (id, t) in m { if let Some(s) = t.as_str() { self.put_text(id, s); } }
            }
            if let Some(m) = b.get("kv").and_then(|x| x.as_object()) {
                for (k, v) in m.iter().filter(|(k, _)| !MACHINE_KV.contains(&k.as_str())) { self.kv_set(k, v); }
            }
            // The search index is rebuilt from the restored topics.
            for t in self.all::<crate::types::Topic>("topics") {
                let text = self.get_text(&format!("topic:{}", t.id)).unwrap_or_default();
                self.index(&t.id, &t.name, &format!("{}\n{}\n{}\n{}", t.summary, t.terms.join(" "), t.unit, text));
            }
        })
    }

    pub fn flush(&self) {
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(PASSIVE)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backups_never_carry_mailbox_state() {
        let dir = std::env::temp_dir().join(format!("is-store-{}", crate::util::uid()));
        std::fs::create_dir_all(&dir).unwrap();
        let st = Store::open(&dir.join("t.sqlite")).unwrap();
        st.kv_set("routinePending", &json!([{ "id": "3f2a9c1e7b4d" }]));
        st.kv_set("jobSeq", &1.0);
        let dump = st.dump();
        assert!(dump["kv"].get("routinePending").is_none() && dump["kv"]["jobSeq"] == json!(1.0));
        // A crafted backup's batch list is ignored and this Mac's own is kept.
        let planted = json!({ "app": "interleaved-study", "colls": {}, "kv": { "routinePending": [{ "id": "../README.md#", "repo": "victim/project" }], "jobSeq": 2.0 } });
        st.restore(&planted);
        assert_eq!(st.kv_get::<Value>("routinePending"), Some(json!([{ "id": "3f2a9c1e7b4d" }])));
        assert_eq!(st.kv_get::<f64>("jobSeq"), Some(2.0));
        drop(st);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
