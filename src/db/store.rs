use std::{path::Path, sync::Arc};

use libsql::{Builder, Database, Value};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A single stored memory row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub agent_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Wraps the libsql Database; each operation opens a fresh Connection so this
/// type is cheaply Clone without any mutex contention.
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
        agent_id: &str,
        embedding: &[f32],
    ) -> Result<i64> {
        let conn = self.conn()?;
        let tags_json = serde_json::to_string(tags)?;
        let now = unix_secs();
        let emb_json = floats_to_json(embedding);

        let mut rows = conn
            .query(
                "INSERT INTO memories
                     (content, summary, tags, agent_id, embedding, created_at, updated_at)
                 VALUES (?, ?, ?, ?, vector32(?), ?, ?)
                 RETURNING rowid",
                libsql::params![content, summary, tags_json, agent_id, emb_json, now, now],
            )
            .await?;

        let row = rows
            .next()
            .await?
            .ok_or_else(|| Error::Internal("INSERT RETURNING yielded no row".into()))?;
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
                    "SELECT rowid, content, summary, tags, agent_id,
                            created_at, updated_at,
                            vector_distance_cos(embedding, vector32(?)) AS dist
                     FROM memories
                     WHERE agent_id = ?
                     ORDER BY dist ASC
                     LIMIT ?",
                    libsql::params![emb_json, aid, limit as i64],
                )
                .await?
            }
            None => {
                conn.query(
                    "SELECT rowid, content, summary, tags, agent_id,
                            created_at, updated_at,
                            vector_distance_cos(embedding, vector32(?)) AS dist
                     FROM memories
                     ORDER BY dist ASC
                     LIMIT ?",
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
                    "SELECT rowid, content, summary, tags, agent_id, created_at, updated_at
                     FROM memories
                     WHERE agent_id = ?
                     ORDER BY created_at DESC
                     LIMIT ? OFFSET ?",
                    libsql::params![aid, limit as i64, offset as i64],
                )
                .await?
            }
            None => {
                conn.query(
                    "SELECT rowid, content, summary, tags, agent_id, created_at, updated_at
                     FROM memories
                     ORDER BY created_at DESC
                     LIMIT ? OFFSET ?",
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
            .execute("DELETE FROM memories WHERE rowid = ?", libsql::params![id])
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
            "UPDATE memories SET {} WHERE rowid = ?",
            set_clauses.join(", ")
        );

        let affected = conn
            .execute(&sql, libsql::params_from_iter(params))
            .await?;
        Ok(affected > 0)
    }
}

// Parses columns 0-6 from a memories SELECT row (rowid first, then schema cols).
fn parse_memory_row(row: &libsql::Row) -> Result<Memory> {
    let id: i64 = row.get(0)?;
    let content: String = row.get(1)?;
    let summary = match row.get_value(2)? {
        Value::Null => None,
        Value::Text(s) => Some(s),
        other => {
            return Err(Error::Internal(format!(
                "unexpected type for summary: {other:?}"
            )))
        }
    };
    let tags_json: String = row.get(3)?;
    let agent_id: String = row.get(4)?;
    let created_at: i64 = row.get(5)?;
    let updated_at: i64 = row.get(6)?;

    let tags: Vec<String> = serde_json::from_str(&tags_json)?;

    Ok(Memory { id, content, summary, tags, agent_id, created_at, updated_at })
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

fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
