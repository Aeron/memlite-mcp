use std::{path::Path, sync::Arc};

use libsql::{Builder, Database, Value};
use serde::{Deserialize, Serialize};

use anyhow::{Result, anyhow};

/// A single stored memory row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub agent: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Wraps the [`libsql::Database`]; each operation opens a fresh [`libsql::Connection`] so this
/// type is cheaply [`Clone`] without any mutex contention.
#[derive(Clone)]
pub struct MemoryStore {
    db: Arc<Database>,
}

impl MemoryStore {
    /// Opens (or creates) a local DB at `path`, or an in-memory DB when None.
    pub async fn open(path: Option<&Path>) -> Result<Self> {
        let db = match path {
            Some(p) => {
                if let Some(parent) = p.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                Builder::new_local(p).build().await?
            }
            None => Builder::new_local(":memory:").build().await?,
        };
        let conn = db.connect()?;
        super::schema::init(&conn).await?;
        Ok(MemoryStore { db: Arc::new(db) })
    }

    fn conn(&self) -> Result<libsql::Connection> {
        Ok(self.db.connect()?)
    }

    /// Inserts a new memory and returns its rowid.
    pub async fn insert(
        &self,
        content: &str,
        summary: Option<&str>,
        tags: &[String],
        agent: &str,
        embedding: &[f32],
    ) -> Result<i64> {
        let conn = self.conn()?;
        let tags_json = serde_json::to_string(tags)?;
        let now = unix_secs();
        let emb_json = floats_to_json(embedding);

        let mut rows = conn
            .query(
                "insert into memories
                     (content, summary, tags, agent, embedding, created_at, updated_at)
                 values (?, ?, ?, ?, vector32(?), ?, ?)
                 returning rowid",
                libsql::params![content, summary, tags_json, agent, emb_json, now, now],
            )
            .await?;

        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow!("INSERT RETURNING yielded no row"))?;
        Ok(row.get(0)?)
    }

    /// Returns memories ordered by cosine distance to `query_embedding`.
    ///
    /// Filters by `agent_id` when non-None.
    pub async fn search(
        &self,
        query_embedding: &[f32],
        agent_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<(Memory, f64)>> {
        let conn = self.conn()?;
        let emb_json = floats_to_json(query_embedding);

        let mut rows = match agent_id {
            Some(aid) => {
                conn.query(
                    "select rowid, content, summary, tags, agent,
                            created_at, updated_at,
                            vector_distance_cos(embedding, vector32(?)) as dist
                     from memories
                     where agent = ?
                     order by dist asc
                     limit ?",
                    libsql::params![emb_json, aid, limit as i64],
                )
                .await?
            }
            None => {
                conn.query(
                    "select rowid, content, summary, tags, agent,
                            created_at, updated_at,
                            vector_distance_cos(embedding, vector32(?)) as dist
                     from memories
                     order by dist asc
                     limit ?",
                    libsql::params![emb_json, limit as i64],
                )
                .await?
            }
        };

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            let memory = parse_memory_row(&row)?;
            let dist: f64 = row.get(7)?;
            results.push((memory, dist));
        }
        Ok(results)
    }

    /// Returns memories ordered by creation time, newest first.
    ///
    /// Filters by `agent_id` when non-None.
    pub async fn list(
        &self,
        agent_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Memory>> {
        let conn = self.conn()?;

        let mut rows = match agent_id {
            Some(aid) => {
                conn.query(
                    "select rowid, content, summary, tags, agent, created_at, updated_at
                     from memories
                     where agent = ?
                     order by created_at desc
                     limit ? offset ?",
                    libsql::params![aid, limit as i64, offset as i64],
                )
                .await?
            }
            None => {
                conn.query(
                    "select rowid, content, summary, tags, agent, created_at, updated_at
                     from memories
                     order by created_at desc
                     limit ? offset ?",
                    libsql::params![limit as i64, offset as i64],
                )
                .await?
            }
        };

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(parse_memory_row(&row)?);
        }
        Ok(results)
    }

    /// Deletes a memory by rowid; returns true if a row was actually removed.
    pub async fn delete(&self, id: i64) -> Result<bool> {
        let conn = self.conn()?;
        let affected = conn
            .execute("delete from memories where rowid = ?", libsql::params![id])
            .await?;
        Ok(affected > 0)
    }

    /// Updates the specified fields of a memory row.
    ///
    /// - `summary = Some("")` clears the summary (sets to NULL).
    /// - Supplying a new `embedding` alongside updated content keeps the index
    ///   consistent; pass None to leave the stored vector unchanged.
    /// - Returns true when at least one row was modified.
    pub async fn update(
        &self,
        id: i64,
        content: Option<&str>,
        summary: Option<&str>,
        tags: Option<&[String]>,
        embedding: Option<&[f32]>,
    ) -> Result<bool> {
        if content.is_none() && summary.is_none() && tags.is_none() && embedding.is_none() {
            return Ok(false);
        }

        let conn = self.conn()?;
        let now = unix_secs();

        let mut set_clauses: Vec<&str> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        if let Some(c) = content {
            set_clauses.push("content = ?");
            params.push(Value::Text(c.to_owned()));
        }
        if let Some(s) = summary {
            set_clauses.push("summary = ?");
            params.push(if s.is_empty() {
                Value::Null
            } else {
                Value::Text(s.to_owned())
            });
        }
        if let Some(t) = tags {
            set_clauses.push("tags = ?");
            params.push(Value::Text(serde_json::to_string(t)?));
        }
        if let Some(emb) = embedding {
            set_clauses.push("embedding = vector32(?)");
            params.push(Value::Text(floats_to_json(emb)));
        }
        set_clauses.push("updated_at = ?");
        params.push(Value::Integer(now));
        params.push(Value::Integer(id));

        let sql = format!(
            "update memories set {} where rowid = ?",
            set_clauses.join(", ")
        );

        let affected = conn.execute(&sql, libsql::params_from_iter(params)).await?;
        Ok(affected > 0)
    }
}

/// Parses a row from the memories table into a [`Memory`] struct.
fn parse_memory_row(row: &libsql::Row) -> Result<Memory> {
    let id: i64 = row.get(0)?;
    let content: String = row.get(1)?;
    let summary = match row.get_value(2)? {
        Value::Null => None,
        Value::Text(s) => Some(s),
        other => return Err(anyhow!("unexpected type for summary: {other:?}")),
    };
    let tags_json: String = row.get(3)?;
    let agent: String = row.get(4)?;
    let created_at: i64 = row.get(5)?;
    let updated_at: i64 = row.get(6)?;

    let tags: Vec<String> = serde_json::from_str(&tags_json)?;

    Ok(Memory {
        id,
        content,
        summary,
        tags,
        agent,
        created_at,
        updated_at,
    })
}

/// Serializes a float slice to a JSON array string accepted by vector32().
fn floats_to_json(data: &[f32]) -> String {
    let mut out = String::with_capacity(data.len() * 12 + 2);
    out.push('[');
    for (i, f) in data.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&f.to_string());
    }
    out.push(']');
    out
}

/// Returns the current Unix timestamp in seconds.
fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    // SQLite :memory: creates a fresh DB per connection, so schema init and
    // queries would hit different databases. A real temp file avoids this.
    async fn fresh() -> (MemoryStore, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = MemoryStore::open(Some(tmp.path())).await.unwrap();
        (store, tmp)
    }

    // 2-D unit vectors used throughout: cheap, no model download needed
    const V1: [f32; 2] = [1.0, 0.0];
    const V2: [f32; 2] = [0.0, 1.0];

    #[test]
    fn floats_to_json_empty() {
        assert_eq!(floats_to_json(&[]), "[]");
    }

    #[test]
    fn floats_to_json_roundtrips() {
        let vals = [1.0f32, 0.5, -0.25];
        let json = floats_to_json(&vals);
        let parsed: Vec<f32> = serde_json::from_str(&json).unwrap();
        for (a, b) in vals.iter().zip(&parsed) {
            assert!((a - b).abs() < 1e-6, "{a} != {b}");
        }
    }

    #[tokio::test]
    async fn insert_returns_positive_id() {
        let (store, _tmp) = fresh().await;
        let id = store.insert("hello", None, &[], "", &V1).await.unwrap();
        assert!(id > 0);
    }

    #[tokio::test]
    async fn list_empty_store() {
        let (store, _tmp) = fresh().await;
        let rows = store.list(None, 10, 0).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn insert_and_list_roundtrip() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("content", Some("summ"), &["tag".into()], "bot", &V1)
            .await
            .unwrap();
        let rows = store.list(None, 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        let m = &rows[0];
        assert_eq!(m.id, id);
        assert_eq!(m.content, "content");
        assert_eq!(m.summary.as_deref(), Some("summ"));
        assert_eq!(m.tags, ["tag"]);
        assert_eq!(m.agent, "bot");
    }

    #[tokio::test]
    async fn list_filters_by_agent() {
        let (store, _tmp) = fresh().await;
        store.insert("a", None, &[], "alice", &V1).await.unwrap();
        store.insert("b", None, &[], "bob", &V1).await.unwrap();

        let alice = store.list(Some("alice"), 10, 0).await.unwrap();
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].agent, "alice");

        let all = store.list(None, 10, 0).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_paginates_without_overlap() {
        let (store, _tmp) = fresh().await;
        for i in 0..5 {
            store
                .insert(&format!("item {i}"), None, &[], "", &V1)
                .await
                .unwrap();
        }
        let page0 = store.list(None, 3, 0).await.unwrap();
        let page1 = store.list(None, 3, 3).await.unwrap();
        assert_eq!(page0.len(), 3);
        assert_eq!(page1.len(), 2);
        let ids0: Vec<i64> = page0.iter().map(|m| m.id).collect();
        let ids1: Vec<i64> = page1.iter().map(|m| m.id).collect();
        assert!(ids0.iter().all(|id| !ids1.contains(id)));
    }

    #[tokio::test]
    async fn delete_existing_returns_true() {
        let (store, _tmp) = fresh().await;
        let id = store.insert("x", None, &[], "", &V1).await.unwrap();
        assert!(store.delete(id).await.unwrap());
        assert!(store.list(None, 10, 0).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_missing_returns_false() {
        let (store, _tmp) = fresh().await;
        assert!(!store.delete(999).await.unwrap());
    }

    #[tokio::test]
    async fn update_content() {
        let (store, _tmp) = fresh().await;
        let id = store.insert("old", None, &[], "", &V1).await.unwrap();
        assert!(
            store
                .update(id, Some("new"), None, None, None)
                .await
                .unwrap()
        );
        assert_eq!(store.list(None, 1, 0).await.unwrap()[0].content, "new");
    }

    #[tokio::test]
    async fn update_clears_summary_on_empty_string() {
        let (store, _tmp) = fresh().await;
        let id = store.insert("x", Some("summ"), &[], "", &V1).await.unwrap();
        store.update(id, None, Some(""), None, None).await.unwrap();
        assert!(store.list(None, 1, 0).await.unwrap()[0].summary.is_none());
    }

    #[tokio::test]
    async fn update_missing_row_returns_false() {
        let (store, _tmp) = fresh().await;
        assert!(
            !store
                .update(999, Some("x"), None, None, None)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn update_no_fields_returns_false() {
        let (store, _tmp) = fresh().await;
        let id = store.insert("x", None, &[], "", &V1).await.unwrap();
        assert!(!store.update(id, None, None, None, None).await.unwrap());
    }

    #[tokio::test]
    async fn search_orders_by_cosine_distance() {
        let (store, _tmp) = fresh().await;
        // V1=[1,0] and V2=[0,1] are orthogonal unit vectors; cosine distance
        // between V1 and V1 is 0, between V1 and V2 is 1
        let id1 = store.insert("first", None, &[], "", &V1).await.unwrap();
        let id2 = store.insert("second", None, &[], "", &V2).await.unwrap();

        let results = store.search(&V1, None, 2).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].0.id, id1,
            "nearest result should be the V1 memory"
        );
        assert_eq!(results[1].0.id, id2);
        assert!(
            results[0].1 < results[1].1,
            "closer result must have smaller distance"
        );
    }

    #[tokio::test]
    async fn search_filters_by_agent() {
        let (store, _tmp) = fresh().await;
        store
            .insert("alice", None, &[], "alice", &V1)
            .await
            .unwrap();
        store.insert("bob", None, &[], "bob", &V1).await.unwrap();

        let results = store.search(&V1, Some("alice"), 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.agent, "alice");
    }
}
