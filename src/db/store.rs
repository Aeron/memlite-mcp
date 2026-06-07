use std::{path::Path, sync::Arc};

use libsql::{Builder, Database, Value};
use serde::{Deserialize, Serialize};

use anyhow::{Result, anyhow};

/// Classifies a memory's intended use, enabling targeted filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Facts about the person: preferences, identity, working style.
    User,
    /// Codebase knowledge: conventions, tech stack, architecture decisions, constraints.
    Project,
    /// Domain knowledge: docs, external facts, API details, terminology.
    Reference,
    /// Things to track, follow up on, or complete in a future session.
    Task,
    /// Discovered problem, audit result, or identified risk; not yet a tracked task.
    Finding,
    /// Known defect or bug.
    Issue,
    /// Architectural or design decision record: what was chosen, what rejected, and why.
    Decision,
    /// High-level plan or phased roadmap; links to tasks and decisions via relations.
    Plan,
}

impl MemoryType {
    /// Returns the lowercase string representation of the variant.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
            MemoryType::Task => "task",
            MemoryType::Finding => "finding",
            MemoryType::Issue => "issue",
            MemoryType::Decision => "decision",
            MemoryType::Plan => "plan",
        }
    }

    /// Parses a lowercase string into the matching variant, returning None on mismatch.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "user" => Some(MemoryType::User),
            "project" => Some(MemoryType::Project),
            "reference" => Some(MemoryType::Reference),
            "task" => Some(MemoryType::Task),
            "finding" => Some(MemoryType::Finding),
            "issue" => Some(MemoryType::Issue),
            "decision" => Some(MemoryType::Decision),
            "plan" => Some(MemoryType::Plan),
            _ => None,
        }
    }
}

/// Tracks the lifecycle state of a memory; most useful for task and issue types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    /// Active and unresolved; the default when set without further qualification.
    Open,
    /// Completed or fixed; no longer requires action.
    Resolved,
    /// Acknowledged but will not be addressed.
    Wontfix,
    /// Postponed for later consideration.
    Deferred,
}

impl MemoryStatus {
    /// Returns the lowercase string representation of the variant.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Open => "open",
            MemoryStatus::Resolved => "resolved",
            MemoryStatus::Wontfix => "wontfix",
            MemoryStatus::Deferred => "deferred",
        }
    }

    /// Parses a lowercase string into the matching variant, returning None on mismatch.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(MemoryStatus::Open),
            "resolved" => Some(MemoryStatus::Resolved),
            "wontfix" => Some(MemoryStatus::Wontfix),
            "deferred" => Some(MemoryStatus::Deferred),
            _ => None,
        }
    }
}

/// Indicates the urgency or impact of a finding, issue, or task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemorySeverity {
    /// Requires immediate attention; may block work or cause data loss.
    Critical,
    /// Significant impact; should be addressed soon.
    High,
    /// Noticeable impact; can be scheduled.
    Medium,
    /// Minor impact; address when convenient.
    Low,
}

impl MemorySeverity {
    /// Returns the lowercase string representation of the variant.
    pub fn as_str(self) -> &'static str {
        match self {
            MemorySeverity::Critical => "critical",
            MemorySeverity::High => "high",
            MemorySeverity::Medium => "medium",
            MemorySeverity::Low => "low",
        }
    }

    /// Parses a lowercase string into the matching variant, returning None on mismatch.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "critical" => Some(MemorySeverity::Critical),
            "high" => Some(MemorySeverity::High),
            "medium" => Some(MemorySeverity::Medium),
            "low" => Some(MemorySeverity::Low),
            _ => None,
        }
    }
}

/// Represents a single stored memory row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Row identifier assigned by the database.
    pub id: i64,
    /// Full text of the memory; the field that is embedded for semantic search.
    pub content: String,
    /// One-sentence description for quick recall without reading the full content.
    pub summary: Option<String>,
    /// Category labels attached to this memory for grouping and filtering.
    pub tags: Vec<String>,
    /// Identifier of the agent that created this memory.
    pub agent: String,
    /// Unix timestamp (seconds) when the row was first inserted.
    pub created_at: i64,
    /// Unix timestamp (seconds) when the row was last modified.
    pub updated_at: i64,
    /// Background rationale: why this memory matters, how decisions were reached, etc.
    pub context: Option<String>,
    /// Base confidence score in [0, 1]; 1.0 means fully trusted.
    pub confidence: f64,
    /// Per-day confidence decay rate; 0.0 means no decay.
    pub decay_rate: f64,
    /// Unix timestamp (seconds) when it was last seen (or confidence validated).
    /// 0 means the created_at timestamp is used as the baseline.
    pub last_seen_at: i64,
    /// Optional classification of the memory's origin or intended use.
    pub memory_type: Option<MemoryType>,
    /// Optional lifecycle state; most meaningful for task and issue memories.
    pub status: Option<MemoryStatus>,
    /// Optional urgency or impact level; most meaningful for finding, issue, and task memories.
    pub severity: Option<MemorySeverity>,
    /// Outgoing directed links keyed by stringified target identifier with a free-form kind label.
    pub relations: std::collections::BTreeMap<String, String>,
}

impl Memory {
    /// Computes the current effective confidence after applying time-based decay.
    ///
    /// Returns `confidence` unchanged when `decay_rate` is zero or non-positive.
    /// Falls back to `created_at` as the validation baseline when `last_seen_at` is 0.
    /// Result is clamped to [0.0, 1.0].
    pub fn effective_confidence(&self) -> f64 {
        if self.decay_rate <= 0.0 {
            return self.confidence.clamp(0.0, 1.0);
        }
        let seen_at = if self.last_seen_at > 0 {
            self.last_seen_at
        } else {
            self.created_at
        };
        let now = unix_secs();
        let days = ((now - seen_at).max(0) as f64) / 86400.0;
        (self.confidence - self.decay_rate * days).clamp(0.0, 1.0)
    }

    /// Returns the number of whole days since this memory was last modified.
    pub fn age_days(&self) -> u64 {
        let now = unix_secs();
        ((now - self.updated_at).max(0) as u64) / 86400
    }

    /// Returns a plain-English warning when this memory may be unreliable.
    ///
    /// Triggers when decay is active and effective confidence has fallen below
    /// the initial value, or when the memory has not been updated in more than
    /// 30 days.
    pub fn freshness_warning(&self) -> Option<String> {
        let eff = self.effective_confidence();

        if self.decay_rate > 0.0 && eff < self.confidence.clamp(0.0, 1.0) {
            let seen_at = if self.last_seen_at > 0 {
                self.last_seen_at
            } else {
                self.created_at
            };
            let val_days = ((unix_secs() - seen_at).max(0) as u64) / 86400;
            return Some(format!(
                "Confidence decayed to {:.0}% (from {:.0}%; {} days since last validation). \
                 Call memory_update to revalidate and reset the decay clock.",
                eff * 100.0,
                self.confidence.clamp(0.0, 1.0) * 100.0,
                val_days,
            ));
        }

        let days = self.age_days();
        if days > 30 {
            return Some(format!(
                "Memory is {} days old; verify it is still current before acting on it.",
                days,
            ));
        }

        None
    }
}

/// Holds the optional metadata fields for inserting a new memory row.
#[derive(Default)]
pub struct InsertOptions<'a> {
    /// Short label for list views.
    pub summary: Option<&'a str>,
    /// Category labels stored as a JSON array.
    pub tags: &'a [String],
    /// Agent identifier; empty string means no specific agent.
    pub agent: &'a str,
    /// Background rationale for the memory.
    pub context: Option<&'a str>,
    /// Base trust score in [0.0, 1.0]; defaults to 1.0.
    pub confidence: Option<f64>,
    /// Per-day confidence decay in [0.0, 1.0]; defaults to 0.0.
    pub decay_rate: Option<f64>,
    /// Optional classification of the memory's origin or intended use.
    pub memory_type: Option<MemoryType>,
    /// Optional lifecycle status; defaults to NULL (no status).
    pub status: Option<MemoryStatus>,
    /// Optional urgency or impact level; defaults to NULL.
    pub severity: Option<MemorySeverity>,
    /// Outgoing directed links as (target id, kind) pairs; defaults to empty.
    pub relations: &'a [(i64, String)],
}

/// Holds the fields that can be updated on an existing memory row.
#[derive(Default)]
pub struct UpdateOptions<'a> {
    /// Replacement content; triggers re-embedding when provided.
    pub content: Option<&'a str>,
    /// Replacement summary; pass Some("") to clear.
    pub summary: Option<&'a str>,
    /// Replacement tags.
    pub tags: Option<&'a [String]>,
    /// Replacement embedding vector.
    pub embedding: Option<&'a [f32]>,
    /// Replacement context; pass Some("") to clear.
    pub context: Option<&'a str>,
    /// Replacement confidence score.
    pub confidence: Option<f64>,
    /// Replacement per-day decay rate.
    pub decay_rate: Option<f64>,
    /// Replacement memory type; set to NULL when `clear_memory_type` is true.
    pub memory_type: Option<MemoryType>,
    /// When true, sets the type column to NULL regardless of `memory_type`.
    pub clear_memory_type: bool,
    /// Replacement lifecycle status; set to NULL when `clear_status` is true.
    pub status: Option<MemoryStatus>,
    /// When true, sets the status column to NULL regardless of `status`.
    pub clear_status: bool,
    /// Replacement severity level; set to NULL when `clear_severity` is true.
    pub severity: Option<MemorySeverity>,
    /// When true, sets the severity column to NULL regardless of `severity`.
    pub clear_severity: bool,
    /// Replacement outgoing relations as (target id, kind) pairs; replaces the full set when provided.
    pub relations: Option<&'a [(i64, String)]>,
}

/// Shared pre-filter options for search and list queries.
#[derive(Default)]
pub struct FilterOptions<'a> {
    /// Restricts results to those owned by this agent identifier.
    pub agent_id: Option<&'a str>,
    /// Restricts results to those carrying all of the given tags (hard AND).
    pub tags: Option<&'a [String]>,
    /// Restricts results to the given memory type.
    pub memory_type: Option<MemoryType>,
    /// Restricts results to the given lifecycle status.
    pub status: Option<MemoryStatus>,
    /// Restricts results to the given severity level.
    pub severity: Option<MemorySeverity>,
}

/// Holds owned data for a single row in a batch insert operation.
pub struct BatchInsertItem {
    /// Full text to embed and associate with this memory.
    pub content: String,
    /// Pre-computed embedding vector for `content`.
    pub embedding: Vec<f32>,
    /// Short label for list views.
    pub summary: Option<String>,
    /// Category labels.
    pub tags: Vec<String>,
    /// Agent identifier.
    pub agent: String,
    /// Background rationale.
    pub context: Option<String>,
    /// Base trust score in [0.0, 1.0]; caller is responsible for clamping.
    pub confidence: f64,
    /// Per-day confidence decay in [0.0, 1.0]; caller is responsible for clamping.
    pub decay_rate: f64,
    /// Optional memory type classification.
    pub memory_type: Option<MemoryType>,
    /// Optional lifecycle status.
    pub status: Option<MemoryStatus>,
    /// Optional urgency or impact level.
    pub severity: Option<MemorySeverity>,
    /// Outgoing directed links as (target id, kind) pairs.
    pub relations: Vec<(i64, String)>,
}

/// Wraps the [`libsql::Database`]; each operation opens a fresh [`libsql::Connection`] so this
/// type is cheaply [`Clone`] without any mutex contention.
#[derive(Clone)]
pub struct MemoryStore {
    db: Arc<Database>,
    // Held only in in-memory mode; all operations reuse this one connection
    // because SQLite :memory: databases are not shared across connections.
    mem: Option<Arc<tokio::sync::Mutex<libsql::Connection>>>,
}

// Owned connection handle for a single store operation.
//
// File mode gets a fresh connection from the pool; memory mode locks
// and holds the shared connection for the duration of the operation.
enum ConnHandle {
    File(libsql::Connection),
    Memory(tokio::sync::OwnedMutexGuard<libsql::Connection>),
}

impl std::ops::Deref for ConnHandle {
    type Target = libsql::Connection;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::File(c) => c,
            Self::Memory(g) => g,
        }
    }
}

impl MemoryStore {
    /// Opens (or creates) a local DB at `path`, or an in-memory DB when None.
    ///
    /// In-memory mode retains the connection on which migrations ran and reuses
    /// it for all subsequent operations, because SQLite `:memory:` databases
    /// are not shared across connections.
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
        super::migrations::run(&conn).await?;
        let mem = if path.is_none() {
            Some(Arc::new(tokio::sync::Mutex::new(conn)))
        } else {
            None
        };
        Ok(MemoryStore {
            db: Arc::new(db),
            mem,
        })
    }

    async fn conn_handle(&self) -> Result<ConnHandle> {
        match &self.mem {
            Some(arc) => Ok(ConnHandle::Memory(Arc::clone(arc).lock_owned().await)),
            None => Ok(ConnHandle::File(self.db.connect()?)),
        }
    }

    /// Validates that `model_name` and `dims` match what is stored in the `meta` table.
    ///
    /// On first call for a given database file, records the model so future opens can
    /// detect mismatches. Returns an error when the stored model differs, preventing
    /// mixed-dimension vectors from corrupting cosine-distance results.
    /// Skips validation for in-memory stores (ephemeral; no persistence across restarts).
    pub async fn check_embedding_model(&self, model_name: &str, dims: usize) -> Result<()> {
        if self.mem.is_some() {
            return Ok(());
        }
        let conn = self.conn_handle().await?;
        let mut rows = conn
            .query("select value from meta where key = 'embedding_model'", ())
            .await?;
        if let Some(row) = rows.next().await? {
            let stored: String = row.get(0)?;
            if stored != model_name {
                return Err(anyhow::anyhow!(
                    "embedding model mismatch: DB was created with '{}' ({} dims) \
                     but '--embedding-model {}' ({} dims) was requested; \
                     use the original model or delete the database to start fresh",
                    stored,
                    self.stored_dims(&conn).await.unwrap_or(0),
                    model_name,
                    dims,
                ));
            }
        } else {
            conn.execute(
                "insert into meta (key, value) values ('embedding_model', ?)",
                libsql::params![model_name],
            )
            .await?;
            conn.execute(
                "insert into meta (key, value) values ('embedding_dims', ?)",
                libsql::params![dims.to_string()],
            )
            .await?;
        }
        Ok(())
    }

    // Reads the stored embedding dimension from meta; used in mismatch error messages.
    async fn stored_dims(&self, conn: &libsql::Connection) -> Result<usize> {
        let mut rows = conn
            .query("select value from meta where key = 'embedding_dims'", ())
            .await?;
        if let Some(row) = rows.next().await? {
            let s: String = row.get(0)?;
            Ok(s.parse().unwrap_or(0))
        } else {
            Ok(0)
        }
    }

    /// Inserts a new memory and returns its rowid.
    ///
    /// `confidence` defaults to 1.0 and `decay_rate` defaults to 0.0 when None.
    /// Both are clamped to [0.0, 1.0].
    pub async fn insert(
        &self,
        content: &str,
        embedding: &[f32],
        opts: InsertOptions<'_>,
    ) -> Result<i64> {
        let InsertOptions {
            summary,
            tags,
            agent,
            context,
            confidence,
            decay_rate,
            memory_type,
            status,
            severity,
            relations,
        } = opts;
        let conn = self.conn_handle().await?;
        let tags_json = serde_json::to_string(tags)?;
        let now = unix_secs();
        let emb_blob = floats_to_blob(embedding);
        let confidence = confidence.unwrap_or(1.0).clamp(0.0, 1.0);
        let decay_rate = decay_rate.unwrap_or(0.0).clamp(0.0, 1.0);

        let mut rows = conn
            .query(
                "insert into memories
                    (content, summary, tags, agent, embedding, created_at, updated_at, context,
                     confidence, decay_rate, last_seen_at, type, status, severity, relations)
                values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                returning rowid",
                libsql::params_from_iter([
                    Value::Text(content.to_owned()),
                    summary
                        .map(|s| Value::Text(s.to_owned()))
                        .unwrap_or(Value::Null),
                    Value::Text(tags_json),
                    Value::Text(agent.to_owned()),
                    Value::Blob(emb_blob),
                    Value::Integer(now),
                    Value::Integer(now),
                    context
                        .map(|c| Value::Text(c.to_owned()))
                        .unwrap_or(Value::Null),
                    Value::Real(confidence),
                    Value::Real(decay_rate),
                    Value::Integer(now),
                    memory_type
                        .map(|mt| Value::Text(mt.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                    status
                        .map(|s| Value::Text(s.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                    severity
                        .map(|s| Value::Text(s.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                    {
                        let map: std::collections::BTreeMap<String, &str> = relations
                            .iter()
                            .map(|(id, kind)| (id.to_string(), kind.as_str()))
                            .collect();
                        Value::Text(serde_json::to_string(&map)?)
                    },
                ]),
            )
            .await?;

        let row = rows
            .next()
            .await?
            .ok_or_else(|| anyhow!("insert returning yielded no row"))?;
        let id: i64 = row.get(0)?;

        // Syncing normalized tag table
        for tag in tags.iter().filter(|t| !t.is_empty()) {
            conn.execute(
                "insert or ignore into memory_tags (memory_id, tag) values (?, ?)",
                libsql::params![id, tag.as_str()],
            )
            .await?;
        }
        Ok(id)
    }

    /// Inserts multiple memories in a single transaction, returning their rowids in input order.
    ///
    /// All rows are inserted atomically: if any insert fails the transaction is rolled back
    /// and no rows are written. `confidence` and `decay_rate` on each item must already be
    /// clamped to [0.0, 1.0] by the caller.
    pub async fn insert_batch(&self, items: Vec<BatchInsertItem>) -> Result<Vec<i64>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.conn_handle().await?;
        let tx = conn.transaction().await?;
        let now = unix_secs();
        let sql = "insert into memories
                (content, summary, tags, agent, embedding, created_at, updated_at, context,
                 confidence, decay_rate, last_seen_at, type, status, severity, relations)
            values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";

        let mut ids = Vec::with_capacity(items.len());
        for item in &items {
            let tags_json = serde_json::to_string(&item.tags)?;
            let emb_blob = floats_to_blob(&item.embedding);
            tx.execute(
                sql,
                libsql::params_from_iter([
                    Value::Text(item.content.clone()),
                    item.summary
                        .as_deref()
                        .map(|s| Value::Text(s.to_owned()))
                        .unwrap_or(Value::Null),
                    Value::Text(tags_json),
                    Value::Text(item.agent.clone()),
                    Value::Blob(emb_blob),
                    Value::Integer(now),
                    Value::Integer(now),
                    item.context
                        .as_deref()
                        .map(|c| Value::Text(c.to_owned()))
                        .unwrap_or(Value::Null),
                    Value::Real(item.confidence),
                    Value::Real(item.decay_rate),
                    Value::Integer(now),
                    item.memory_type
                        .map(|mt| Value::Text(mt.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                    item.status
                        .map(|s| Value::Text(s.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                    item.severity
                        .map(|s| Value::Text(s.as_str().to_owned()))
                        .unwrap_or(Value::Null),
                    {
                        let map: std::collections::BTreeMap<String, &str> = item
                            .relations
                            .iter()
                            .map(|(id, kind)| (id.to_string(), kind.as_str()))
                            .collect();
                        Value::Text(serde_json::to_string(&map)?)
                    },
                ]),
            )
            .await?;
            let id = tx.last_insert_rowid();
            ids.push(id);
            // Syncing normalized tag table within the same transaction
            for tag in item.tags.iter().filter(|t| !t.is_empty()) {
                tx.execute(
                    "insert or ignore into memory_tags (memory_id, tag) values (?, ?)",
                    libsql::params![id, tag.as_str()],
                )
                .await?;
            }
        }
        tx.commit().await?;
        Ok(ids)
    }

    /// Returns memories ordered by cosine distance to `query_embedding`.
    ///
    /// Fetches up to `limit * 4` candidates from the database (capped at 400),
    /// then re-ranks by word-overlap between `query_text` and each memory's content.
    /// Tie-breaks on cosine distance (ascending). Returns the top `limit` results.
    ///
    /// Optionally pre-filters by `agent_id` and/or `tags` (all supplied tags must match).
    pub async fn search(
        &self,
        query_text: &str,
        query_embedding: &[f32],
        filter: FilterOptions<'_>,
        limit: u32,
    ) -> Result<Vec<(Memory, f64)>> {
        let conn = self.conn_handle().await?;
        let emb_blob = floats_to_blob(query_embedding);
        let candidate_limit = (limit as i64).saturating_mul(4).min(400);
        let mut params: Vec<Value> = vec![Value::Blob(emb_blob)];
        let where_sql = build_where(
            filter.agent_id,
            filter.tags,
            filter.memory_type,
            filter.status,
            filter.severity,
            &mut params,
        )?;
        params.push(Value::Integer(candidate_limit));

        // dist is placed last so parse_memory_row can use a fixed column range (0-14)
        let sql = format!(
            "select
                rowid, content, summary, tags, agent,
                created_at, updated_at, context,
                confidence, decay_rate, last_seen_at, type, status, severity, relations,
                vector_distance_cos(embedding, ?) as dist
            from memories
            {where_sql}
            order by dist asc
            limit ?"
        );
        let mut rows = conn.query(&sql, libsql::params_from_iter(params)).await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            let memory = parse_memory_row(&row)?;
            let dist: f64 = row.get(15)?;
            results.push((memory, dist));
        }
        Ok(rerank_by_overlap(query_text, results, limit as usize))
    }

    /// Returns memories ordered by creation time, newest first.
    ///
    /// Optionally pre-filters by `agent_id` and/or `tags` (all supplied tags must match).
    pub async fn list(
        &self,
        filter: FilterOptions<'_>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Memory>> {
        let conn = self.conn_handle().await?;
        let mut params: Vec<Value> = Vec::new();
        let where_sql = build_where(
            filter.agent_id,
            filter.tags,
            filter.memory_type,
            filter.status,
            filter.severity,
            &mut params,
        )?;
        params.push(Value::Integer(limit as i64));
        params.push(Value::Integer(offset as i64));

        let sql = format!(
            "select
                rowid, content, summary, tags, agent, created_at, updated_at, context,
                confidence, decay_rate, last_seen_at, type, status, severity, relations
            from memories
            {where_sql}
            order by created_at desc
            limit ? offset ?"
        );
        let mut rows = conn.query(&sql, libsql::params_from_iter(params)).await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(parse_memory_row(&row)?);
        }
        Ok(results)
    }

    /// Returns all distinct tags with occurrence counts, ordered by count descending.
    pub async fn tags(&self, agent_id: Option<&str>) -> Result<Vec<(String, u32)>> {
        let conn = self.conn_handle().await?;
        let mut params: Vec<Value> = Vec::new();
        let where_sql = build_where(agent_id, None, None, None, None, &mut params)?;
        let sql = format!(
            "select jt.value, count(*) as cnt
            from memories, json_each(memories.tags) jt
            {where_sql}
            group by jt.value
            order by cnt desc"
        );
        let mut rows = conn.query(&sql, libsql::params_from_iter(params)).await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            let tag: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            results.push((tag, count as u32));
        }
        Ok(results)
    }

    /// Deletes a memory by rowid; returns true if a row was actually removed.
    ///
    /// When `agent_id` is set, only removes the row if it also matches that agent.
    pub async fn delete(&self, id: i64, agent_id: Option<&str>) -> Result<bool> {
        let conn = self.conn_handle().await?;
        let affected = if let Some(agent) = agent_id {
            conn.execute(
                "delete from memories where rowid = ? and agent = ?",
                libsql::params![id, agent],
            )
            .await?
        } else {
            conn.execute("delete from memories where rowid = ?", libsql::params![id])
                .await?
        };
        // Cleaning up normalized tag rows for the deleted memory
        if affected > 0 {
            conn.execute(
                "delete from memory_tags where memory_id = ?",
                libsql::params![id],
            )
            .await?;
        }
        Ok(affected > 0)
    }

    /// Updates the specified fields of a memory row.
    ///
    /// - `summary = Some("")` clears the summary (sets to NULL).
    /// - `context = Some("")` clears the context (sets to NULL).
    /// - `clear_memory_type/status/severity = true` sets the column to NULL.
    /// - Supplying a new `embedding` alongside updated content keeps the index
    ///   consistent; pass None to leave the stored vector unchanged.
    /// - `confidence` and `decay_rate` are clamped to [0.0, 1.0] when provided.
    /// - `last_seen_at` is always bumped to now when any field is updated,
    ///   resetting the confidence decay clock.
    /// - When `agent_id` is set, only modifies the row if it also matches that agent.
    /// - Returns true when at least one row was modified.
    pub async fn update(
        &self,
        id: i64,
        opts: UpdateOptions<'_>,
        agent_id: Option<&str>,
    ) -> Result<bool> {
        let UpdateOptions {
            content,
            summary,
            tags,
            embedding,
            context,
            confidence,
            decay_rate,
            memory_type,
            clear_memory_type,
            status,
            clear_status,
            severity,
            clear_severity,
            relations,
        } = opts;
        if content.is_none()
            && summary.is_none()
            && tags.is_none()
            && embedding.is_none()
            && context.is_none()
            && confidence.is_none()
            && decay_rate.is_none()
            && memory_type.is_none()
            && !clear_memory_type
            && status.is_none()
            && !clear_status
            && severity.is_none()
            && !clear_severity
            && relations.is_none()
        {
            return Ok(false);
        }

        let conn = self.conn_handle().await?;
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
            set_clauses.push("embedding = ?");
            params.push(Value::Blob(floats_to_blob(emb)));
        }
        if let Some(ctx) = context {
            set_clauses.push("context = ?");
            params.push(if ctx.is_empty() {
                Value::Null
            } else {
                Value::Text(ctx.to_owned())
            });
        }
        if let Some(c) = confidence {
            set_clauses.push("confidence = ?");
            params.push(Value::Real(c.clamp(0.0, 1.0)));
        }
        if let Some(dr) = decay_rate {
            set_clauses.push("decay_rate = ?");
            params.push(Value::Real(dr.clamp(0.0, 1.0)));
        }
        if let Some(mt) = memory_type {
            set_clauses.push("type = ?");
            params.push(Value::Text(mt.as_str().to_owned()));
        } else if clear_memory_type {
            set_clauses.push("type = null");
        }
        if let Some(st) = status {
            set_clauses.push("status = ?");
            params.push(Value::Text(st.as_str().to_owned()));
        } else if clear_status {
            set_clauses.push("status = null");
        }
        if let Some(sev) = severity {
            set_clauses.push("severity = ?");
            params.push(Value::Text(sev.as_str().to_owned()));
        } else if clear_severity {
            set_clauses.push("severity = null");
        }
        if let Some(rels) = relations {
            set_clauses.push("relations = ?");
            let map: std::collections::BTreeMap<String, &str> = rels
                .iter()
                .map(|(id, kind)| (id.to_string(), kind.as_str()))
                .collect();
            params.push(Value::Text(serde_json::to_string(&map)?));
        }
        // Always bump both timestamps: updated_at records the data change,
        // last_seen_at resets the confidence decay clock
        set_clauses.push("updated_at = ?");
        params.push(Value::Integer(now));
        set_clauses.push("last_seen_at = ?");
        params.push(Value::Integer(now));
        params.push(Value::Integer(id));
        let sql = if let Some(agent) = agent_id {
            params.push(Value::Text(agent.to_owned()));
            format!(
                "update memories set {} where rowid = ? and agent = ?",
                set_clauses.join(", ")
            )
        } else {
            format!(
                "update memories set {} where rowid = ?",
                set_clauses.join(", ")
            )
        };

        let affected = conn.execute(&sql, libsql::params_from_iter(params)).await?;
        // Syncing memory_tags when the tags column was updated
        if affected > 0
            && let Some(updated_tags) = tags
        {
            conn.execute(
                "delete from memory_tags where memory_id = ?",
                libsql::params![id],
            )
            .await?;

            for tag in updated_tags.iter().filter(|t| !t.is_empty()) {
                conn.execute(
                    "insert or ignore into memory_tags (memory_id, tag) values (?, ?)",
                    libsql::params![id, tag.as_str()],
                )
                .await?;
            }
        }
        Ok(affected > 0)
    }

    /// Resets the decay clock on a batch of memories without changing any other field.
    ///
    /// Issues a single UPDATE touching only `updated_at` and `last_seen_at`. Returns
    /// the count of rows actually modified; may be less than `ids.len()` when some
    /// ids do not exist. When `agent_id` is set, only touches rows that match that agent.
    pub async fn revalidate_batch(&self, ids: &[i64], agent_id: Option<&str>) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.conn_handle().await?;
        let now = unix_secs();
        let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = if agent_id.is_some() {
            format!(
                "update memories set updated_at = ?, last_seen_at = ? where rowid in ({placeholders}) and agent = ?"
            )
        } else {
            format!(
                "update memories set updated_at = ?, last_seen_at = ? where rowid in ({placeholders})"
            )
        };
        let mut params: Vec<Value> = Vec::with_capacity(3 + ids.len());
        params.push(Value::Integer(now));
        params.push(Value::Integer(now));
        for id in ids {
            params.push(Value::Integer(*id));
        }
        if let Some(agent) = agent_id {
            params.push(Value::Text(agent.to_owned()));
        }
        let affected = conn.execute(&sql, libsql::params_from_iter(params)).await?;
        Ok(affected)
    }
}

// Builds a SQL WHERE clause from optional agent, tag, and type filters, appending
// the corresponding bind params in the correct placeholder order.
fn build_where(
    agent_id: Option<&str>,
    tags: Option<&[String]>,
    type_filter: Option<MemoryType>,
    status_filter: Option<MemoryStatus>,
    severity_filter: Option<MemorySeverity>,
    params: &mut Vec<Value>,
) -> Result<String> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(aid) = agent_id {
        clauses.push("agent = ?".into());
        params.push(Value::Text(aid.to_owned()));
    }
    if let Some(ts) = tags.filter(|t| !t.is_empty()) {
        // Using normalized memory_tags table for indexed filtering
        clauses.push(
            "rowid in (select memory_id from memory_tags \
            where tag in (select value from json_each(?)) \
            group by memory_id having count(distinct tag) = ?)"
                .into(),
        );
        params.push(Value::Text(serde_json::to_string(ts)?));
        params.push(Value::Integer(ts.len() as i64));
    }
    if let Some(mt) = type_filter {
        clauses.push("type = ?".into());
        params.push(Value::Text(mt.as_str().to_owned()));
    }
    if let Some(sf) = status_filter {
        clauses.push("status = ?".into());
        params.push(Value::Text(sf.as_str().to_owned()));
    }
    if let Some(sev) = severity_filter {
        clauses.push("severity = ?".into());
        params.push(Value::Text(sev.as_str().to_owned()));
    }
    Ok(if clauses.is_empty() {
        String::new()
    } else {
        format!("where {}", clauses.join(" and "))
    })
}

/// Deserialises a database row into a [`Memory`] value.
///
/// Expects columns in the order selected by `list` and `search`:
/// 0=rowid, 1=content, 2=summary, 3=tags, 4=agent,
/// 5=created_at, 6=updated_at, 7=context,
/// 8=confidence, 9=decay_rate, 10=last_seen_at, 11=type, 12=status, 13=severity, 14=relations.
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
    let context = match row.get_value(7)? {
        Value::Null => None,
        Value::Text(s) => Some(s),
        other => return Err(anyhow!("unexpected type for context: {other:?}")),
    };
    let confidence: f64 = row.get(8)?;
    let decay_rate: f64 = row.get(9)?;
    let last_seen_at: i64 = row.get(10)?;
    let memory_type = match row.get_value(11)? {
        Value::Null => None,
        Value::Text(s) => MemoryType::from_str(&s),
        other => return Err(anyhow!("unexpected value for type column: {other:?}")),
    };
    let status = match row.get_value(12)? {
        Value::Null => None,
        Value::Text(s) => MemoryStatus::from_str(&s),
        other => return Err(anyhow!("unexpected value for status column: {other:?}")),
    };
    let severity = match row.get_value(13)? {
        Value::Null => None,
        Value::Text(s) => MemorySeverity::from_str(&s),
        other => return Err(anyhow!("unexpected value for severity column: {other:?}")),
    };
    let relations_json: String = row.get(14)?;
    let relations: std::collections::BTreeMap<String, String> =
        serde_json::from_str(&relations_json)?;

    let tags: Vec<String> = serde_json::from_str(&tags_json)?;

    Ok(Memory {
        id,
        content,
        summary,
        tags,
        agent,
        created_at,
        updated_at,
        context,
        confidence,
        decay_rate,
        last_seen_at,
        memory_type,
        status,
        severity,
        relations,
    })
}

/// Tokenises a text string into a deduplicated set of lowercase alphanumeric words (length >= 2).
fn word_set(text: &str) -> std::collections::HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter_map(|w| {
            let w = w.to_lowercase();
            if w.len() >= 2 { Some(w) } else { None }
        })
        .collect()
}

/// Re-ranks candidates by whole-word overlap between the query and each memory's content.
///
/// Splits both query and content into deduplicated word sets so duplicate query words
/// do not inflate the score and substring matches (e.g. "is" inside "this") are excluded.
/// Sorts by overlap fraction descending, breaking ties by cosine distance ascending.
/// Truncates to `limit`. When the query has no usable words the original order is kept.
fn rerank_by_overlap(
    query: &str,
    mut candidates: Vec<(Memory, f64)>,
    limit: usize,
) -> Vec<(Memory, f64)> {
    if limit == 0 {
        return Vec::new();
    }
    let query_words = word_set(query);
    if query_words.is_empty() {
        candidates.truncate(limit);
        return candidates;
    }
    let mut scored: Vec<(Memory, f64, f64)> = candidates
        .into_iter()
        .map(|(m, dist)| {
            let content_words = word_set(&m.content);
            let hits = query_words
                .iter()
                .filter(|w| content_words.contains(*w))
                .count();
            let overlap = hits as f64 / query_words.len() as f64;
            (m, dist, overlap)
        })
        .collect();
    // Higher overlap first; lower cosine distance as tiebreaker
    scored.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(m, dist, _)| (m, dist)).collect()
}

/// Packs a float slice as little-endian IEEE 754 bytes, matching libsql's `float32` vector format.
fn floats_to_blob(data: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 4);
    for &f in data {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Returns the current Unix timestamp in whole seconds.
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

    #[tokio::test]
    async fn in_memory_insert_list_roundtrip() {
        let store = MemoryStore::open(None).await.unwrap();
        let id = store
            .insert("hello", &V1, InsertOptions::default())
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1, "insert must be visible to a subsequent list");
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].content, "hello");
    }

    #[tokio::test]
    async fn in_memory_search_roundtrip() {
        let store = MemoryStore::open(None).await.unwrap();
        store
            .insert("hello", &V1, InsertOptions::default())
            .await
            .unwrap();
        let results = store
            .search("hello", &V1, FilterOptions::default(), 10)
            .await
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "insert must be visible to a subsequent search"
        );
        assert_eq!(results[0].0.content, "hello");
    }

    #[test]
    fn floats_to_blob_empty() {
        assert_eq!(floats_to_blob(&[]), Vec::<u8>::new());
    }

    #[test]
    fn floats_to_blob_roundtrips() {
        let vals = [1.0f32, 0.5, -0.25];
        let blob = floats_to_blob(&vals);
        assert_eq!(blob.len(), vals.len() * 4);
        let parsed: Vec<f32> = blob
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        for (a, b) in vals.iter().zip(&parsed) {
            assert_eq!(a.to_bits(), b.to_bits(), "{a} != {b}");
        }
    }

    #[tokio::test]
    async fn insert_returns_positive_id() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("hello", &V1, InsertOptions::default())
            .await
            .unwrap();
        assert!(id > 0);
    }

    #[tokio::test]
    async fn list_empty_store() {
        let (store, _tmp) = fresh().await;
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn insert_and_list_roundtrip() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "content",
                &V1,
                InsertOptions {
                    summary: Some("summ"),
                    tags: &["tag".into()],
                    agent: "bot",
                    context: Some("why it matters"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        let m = &rows[0];
        assert_eq!(m.id, id);
        assert_eq!(m.content, "content");
        assert_eq!(m.summary.as_deref(), Some("summ"));
        assert_eq!(m.tags, ["tag"]);
        assert_eq!(m.agent, "bot");
        assert_eq!(m.context.as_deref(), Some("why it matters"));
        // Defaults
        assert_eq!(m.confidence, 1.0);
        assert_eq!(m.decay_rate, 0.0);
        assert!(m.last_seen_at > 0);
    }

    #[tokio::test]
    async fn insert_stores_confidence_and_decay_rate() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    confidence: Some(0.7),
                    decay_rate: Some(0.01),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].confidence, 0.7);
        assert_eq!(rows[0].decay_rate, 0.01);
    }

    #[tokio::test]
    async fn list_filters_by_agent() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "a",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "b",
                &V1,
                InsertOptions {
                    agent: "bob",
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let alice = store
            .list(
                FilterOptions {
                    agent_id: Some("alice"),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].agent, "alice");

        let all = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn list_paginates_without_overlap() {
        let (store, _tmp) = fresh().await;
        for i in 0..5 {
            store
                .insert(&format!("item {i}"), &V1, InsertOptions::default())
                .await
                .unwrap();
        }
        let page0 = store.list(FilterOptions::default(), 3, 0).await.unwrap();
        let page1 = store.list(FilterOptions::default(), 3, 3).await.unwrap();
        assert_eq!(page0.len(), 3);
        assert_eq!(page1.len(), 2);
        let ids0: Vec<i64> = page0.iter().map(|m| m.id).collect();
        let ids1: Vec<i64> = page1.iter().map(|m| m.id).collect();
        assert!(ids0.iter().all(|id| !ids1.contains(id)));
    }

    #[tokio::test]
    async fn delete_existing_returns_true() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        assert!(store.delete(id, None).await.unwrap());
        assert!(
            store
                .list(FilterOptions::default(), 10, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn delete_missing_returns_false() {
        let (store, _tmp) = fresh().await;
        assert!(!store.delete(999, None).await.unwrap());
    }

    #[tokio::test]
    async fn list_filters_by_tags() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "a",
                &V1,
                InsertOptions {
                    tags: &["bug".into(), "config".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "b",
                &V1,
                InsertOptions {
                    tags: &["bug".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert("c", &V1, InsertOptions::default())
            .await
            .unwrap();

        let bug_only = store
            .list(
                FilterOptions {
                    tags: Some(&["bug".into()]),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(bug_only.len(), 2);

        let both = store
            .list(
                FilterOptions {
                    tags: Some(&["bug".into(), "config".into()]),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(both.len(), 1);
        assert_eq!(both[0].content, "a");

        // empty slice = no filter
        let all = store
            .list(
                FilterOptions {
                    tags: Some(&[]),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn search_filters_by_tags() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "tagged",
                &V1,
                InsertOptions {
                    tags: &["rust".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert("untagged", &V2, InsertOptions::default())
            .await
            .unwrap();

        let results = store
            .search(
                "",
                &V1,
                FilterOptions {
                    tags: Some(&["rust".into()]),
                    ..Default::default()
                },
                10,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.content, "tagged");
    }

    #[tokio::test]
    async fn tags_returns_counts() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    tags: &["a".into(), "b".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "y",
                &V1,
                InsertOptions {
                    tags: &["a".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert("z", &V1, InsertOptions::default())
            .await
            .unwrap();

        let counts = store.tags(None).await.unwrap();
        assert_eq!(counts.len(), 2);
        // ordered by count desc: "a" (2) before "b" (1)
        assert_eq!(counts[0], ("a".into(), 2));
        assert_eq!(counts[1], ("b".into(), 1));
    }

    #[tokio::test]
    async fn tags_filters_by_agent() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    tags: &["shared".into()],
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "y",
                &V1,
                InsertOptions {
                    tags: &["shared".into(), "private".into()],
                    agent: "bob",
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let alice_tags = store.tags(Some("alice")).await.unwrap();
        assert_eq!(alice_tags.len(), 1);
        assert_eq!(alice_tags[0].0, "shared");
        assert!(!alice_tags.iter().any(|(t, _)| t == "private"));
    }

    #[tokio::test]
    async fn update_content() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("old", &V1, InsertOptions::default())
            .await
            .unwrap();
        assert!(
            store
                .update(
                    id,
                    UpdateOptions {
                        content: Some("new"),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .unwrap()
        );
        assert_eq!(
            store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].content,
            "new"
        );
    }

    #[tokio::test]
    async fn update_clears_summary_on_empty_string() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    summary: Some("summ"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    summary: Some(""),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert!(
            store.list(FilterOptions::default(), 1, 0).await.unwrap()[0]
                .summary
                .is_none()
        );
    }

    #[tokio::test]
    async fn update_clears_context_on_empty_string() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    context: Some("initial context"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    context: Some(""),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert!(
            store.list(FilterOptions::default(), 1, 0).await.unwrap()[0]
                .context
                .is_none()
        );
    }

    #[tokio::test]
    async fn update_missing_row_returns_false() {
        let (store, _tmp) = fresh().await;
        assert!(
            !store
                .update(
                    999,
                    UpdateOptions {
                        content: Some("x"),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn update_no_fields_returns_false() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        assert!(
            !store
                .update(id, UpdateOptions::default(), None)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn update_confidence_and_decay_rate() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        assert!(
            store
                .update(
                    id,
                    UpdateOptions {
                        confidence: Some(0.6),
                        decay_rate: Some(0.02),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .unwrap()
        );
        let m = &store.list(FilterOptions::default(), 1, 0).await.unwrap()[0];
        assert_eq!(m.confidence, 0.6);
        assert_eq!(m.decay_rate, 0.02);
    }

    #[tokio::test]
    async fn update_bumps_last_seen_at() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        let before = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        // Small sleep so the new timestamp is strictly greater
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        store
            .update(
                id,
                UpdateOptions {
                    content: Some("updated"),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        let after = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        assert!(after > before, "last_seen_at should increase after update");
    }

    #[tokio::test]
    async fn effective_confidence_no_decay() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    confidence: Some(0.8),
                    decay_rate: Some(0.0),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let m = &store.list(FilterOptions::default(), 1, 0).await.unwrap()[0];
        assert_eq!(m.effective_confidence(), 0.8);
    }

    #[tokio::test]
    async fn effective_confidence_decays_over_time() {
        // Confidence starts at 1.0 with a very high decay rate so even a
        // sub-second-old memory drops visibly when we fake an old seen_at
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    confidence: Some(1.0),
                    decay_rate: Some(0.5),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Forcing last_seen_at to 30 days ago via a direct update
        let thirty_days_ago = unix_secs() - 30 * 86400;
        let conn = store.conn_handle().await.unwrap();
        conn.execute(
            "update memories set last_seen_at = ? where rowid = ?",
            libsql::params![thirty_days_ago, id],
        )
        .await
        .unwrap();
        let m = &store.list(FilterOptions::default(), 1, 0).await.unwrap()[0];
        // 30 days * 0.5/day = 15 decay: 1.0 - 15 = -14, clamped to 0.0
        assert_eq!(m.effective_confidence(), 0.0);
    }

    #[tokio::test]
    async fn age_days_freshly_inserted() {
        let (store, _tmp) = fresh().await;
        store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        let m = &store.list(FilterOptions::default(), 1, 0).await.unwrap()[0];
        assert_eq!(m.age_days(), 0);
    }

    #[tokio::test]
    async fn search_orders_by_cosine_distance() {
        let (store, _tmp) = fresh().await;
        // V1=[1,0] and V2=[0,1] are orthogonal unit vectors; cosine distance
        // between V1 and V1 is 0, between V1 and V2 is 1
        let id1 = store
            .insert("first", &V1, InsertOptions::default())
            .await
            .unwrap();
        let id2 = store
            .insert("second", &V2, InsertOptions::default())
            .await
            .unwrap();

        // Empty query: no word-overlap re-ranking, pure cosine order preserved
        let results = store
            .search("", &V1, FilterOptions::default(), 2)
            .await
            .unwrap();
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
    async fn search_reranks_by_word_overlap() {
        let (store, _tmp) = fresh().await;
        // V1 is closer to the query vector but the content doesn't match the query words.
        // V2 has matching content and should rank first after re-ranking
        store
            .insert("unrelated content", &V1, InsertOptions::default())
            .await
            .unwrap();
        let id2 = store
            .insert("rust programming language", &V2, InsertOptions::default())
            .await
            .unwrap();

        // Query vector identical to V1 (so cosine order would put the first item first),
        // but query text matches the second item
        let results = store
            .search("rust programming", &V1, FilterOptions::default(), 2)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].0.id, id2,
            "word-overlap re-rank should put the matching item first"
        );
    }

    #[tokio::test]
    async fn search_filters_by_agent() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "alice",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "bob",
                &V1,
                InsertOptions {
                    agent: "bob",
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let results = store
            .search(
                "",
                &V1,
                FilterOptions {
                    agent_id: Some("alice"),
                    ..Default::default()
                },
                10,
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.agent, "alice");
    }

    #[test]
    fn rerank_empty_query_preserves_cosine_order() {
        let make = |content: &str, dist: f64| -> (Memory, f64) {
            (
                Memory {
                    id: 1,
                    content: content.to_owned(),
                    summary: None,
                    tags: vec![],
                    agent: String::new(),
                    created_at: 0,
                    updated_at: 0,
                    context: None,
                    confidence: 1.0,
                    decay_rate: 0.0,
                    last_seen_at: 0,
                    memory_type: None,
                    status: None,
                    severity: None,
                    relations: std::collections::BTreeMap::new(),
                },
                dist,
            )
        };
        let candidates = vec![make("a", 0.1), make("b", 0.5), make("c", 0.9)];
        let ranked = rerank_by_overlap("", candidates, 3);
        assert_eq!(ranked[0].0.content, "a");
        assert_eq!(ranked[1].0.content, "b");
        assert_eq!(ranked[2].0.content, "c");
    }

    #[test]
    fn rerank_overlap_beats_distance() {
        let make = |content: &str, dist: f64| -> (Memory, f64) {
            (
                Memory {
                    id: 1,
                    content: content.to_owned(),
                    summary: None,
                    tags: vec![],
                    agent: String::new(),
                    created_at: 0,
                    updated_at: 0,
                    context: None,
                    confidence: 1.0,
                    decay_rate: 0.0,
                    last_seen_at: 0,
                    memory_type: None,
                    status: None,
                    severity: None,
                    relations: std::collections::BTreeMap::new(),
                },
                dist,
            )
        };
        // "rust tips" is farther (dist 0.9) but has two-word overlap.
        // "python docs" is closer (dist 0.1) but has no overlap
        let candidates = vec![make("python docs", 0.1), make("rust tips guide", 0.9)];
        let ranked = rerank_by_overlap("rust tips", candidates, 2);
        assert_eq!(ranked[0].0.content, "rust tips guide");
        assert_eq!(ranked[1].0.content, "python docs");
    }

    #[tokio::test]
    async fn list_filters_by_memory_type() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "architectural choice",
                &V1,
                InsertOptions {
                    memory_type: Some(MemoryType::Decision),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "known defect",
                &V1,
                InsertOptions {
                    memory_type: Some(MemoryType::Issue),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert("no type", &V1, InsertOptions::default())
            .await
            .unwrap();

        let decisions = store
            .list(
                FilterOptions {
                    memory_type: Some(MemoryType::Decision),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].content, "architectural choice");

        let issues = store
            .list(
                FilterOptions {
                    memory_type: Some(MemoryType::Issue),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].content, "known defect");

        let all = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn update_memory_type() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "general note",
                &V1,
                InsertOptions {
                    memory_type: Some(MemoryType::Reference),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    memory_type: Some(MemoryType::Project),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].memory_type, Some(MemoryType::Project));
    }

    #[tokio::test]
    async fn insert_and_list_roundtrip_with_status() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "fix login crash",
                &V1,
                InsertOptions {
                    status: Some(MemoryStatus::Open),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].status, Some(MemoryStatus::Open));
    }

    #[tokio::test]
    async fn list_filters_by_status() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "open issue",
                &V1,
                InsertOptions {
                    status: Some(MemoryStatus::Open),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "resolved issue",
                &V1,
                InsertOptions {
                    status: Some(MemoryStatus::Resolved),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert("no status", &V1, InsertOptions::default())
            .await
            .unwrap();

        let open = store
            .list(
                FilterOptions {
                    status: Some(MemoryStatus::Open),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].content, "open issue");

        let resolved = store
            .list(
                FilterOptions {
                    status: Some(MemoryStatus::Resolved),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].content, "resolved issue");

        let all = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn update_status() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "track this",
                &V1,
                InsertOptions {
                    status: Some(MemoryStatus::Open),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let updated = store
            .update(
                id,
                UpdateOptions {
                    status: Some(MemoryStatus::Resolved),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert!(updated);

        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].status, Some(MemoryStatus::Resolved));
    }

    #[tokio::test]
    async fn insert_batch_empty_returns_empty() {
        let (store, _tmp) = fresh().await;
        let ids = store.insert_batch(vec![]).await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn insert_batch_returns_ids_in_order() {
        let (store, _tmp) = fresh().await;
        let items = vec![
            BatchInsertItem {
                content: "first".into(),
                embedding: V1.to_vec(),
                summary: Some("s1".into()),
                tags: vec!["a".into()],
                agent: "bot".into(),
                context: None,
                confidence: 1.0,
                decay_rate: 0.0,
                memory_type: Some(MemoryType::Project),
                status: Some(MemoryStatus::Open),
                severity: None,
                relations: vec![],
            },
            BatchInsertItem {
                content: "second".into(),
                embedding: V2.to_vec(),
                summary: None,
                tags: vec![],
                agent: "bot".into(),
                context: None,
                confidence: 0.8,
                decay_rate: 0.01,
                memory_type: None,
                status: Some(MemoryStatus::Resolved),
                severity: None,
                relations: vec![],
            },
        ];
        let ids = store.insert_batch(items).await.unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids[0] > 0);
        assert!(ids[1] > ids[0]);

        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        // list returns newest-first, so second item is at index 0
        assert_eq!(rows.len(), 2);
        let first = rows.iter().find(|m| m.content == "first").unwrap();
        let second = rows.iter().find(|m| m.content == "second").unwrap();
        assert_eq!(first.id, ids[0]);
        assert_eq!(second.id, ids[1]);
        assert_eq!(first.summary.as_deref(), Some("s1"));
        assert_eq!(first.tags, ["a"]);
        assert_eq!(first.memory_type, Some(MemoryType::Project));
        assert_eq!(first.status, Some(MemoryStatus::Open));
        assert_eq!(second.status, Some(MemoryStatus::Resolved));
    }

    #[tokio::test]
    async fn insert_and_list_roundtrip_with_severity() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "sql injection in login",
                &V1,
                InsertOptions {
                    severity: Some(MemorySeverity::Critical),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].severity, Some(MemorySeverity::Critical));
    }

    #[tokio::test]
    async fn list_filters_by_severity() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "critical bug",
                &V1,
                InsertOptions {
                    severity: Some(MemorySeverity::Critical),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert(
                "low priority",
                &V1,
                InsertOptions {
                    severity: Some(MemorySeverity::Low),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .insert("no severity", &V1, InsertOptions::default())
            .await
            .unwrap();

        let critical = store
            .list(
                FilterOptions {
                    severity: Some(MemorySeverity::Critical),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(critical.len(), 1);
        assert_eq!(critical[0].content, "critical bug");

        let all = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn update_severity() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "medium issue",
                &V1,
                InsertOptions {
                    severity: Some(MemorySeverity::Medium),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    severity: Some(MemorySeverity::High),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].severity, Some(MemorySeverity::High));
    }

    #[tokio::test]
    async fn insert_and_list_roundtrip_with_relations() {
        let (store, _tmp) = fresh().await;
        // Insert a target memory first so relation ids are valid in spirit
        let target_id = store
            .insert("target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let id = store
            .insert(
                "source with links",
                &V1,
                InsertOptions {
                    relations: &[
                        (target_id, "caused_by".to_owned()),
                        (99, "related_to".to_owned()),
                    ],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        let source = rows.iter().find(|m| m.id == id).unwrap();
        let mut expected = std::collections::BTreeMap::new();
        expected.insert(target_id.to_string(), "caused_by".to_owned());
        expected.insert("99".to_owned(), "related_to".to_owned());
        assert_eq!(source.relations, expected);
    }

    #[tokio::test]
    async fn insert_with_empty_relations_roundtrips() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("no links", &V1, InsertOptions::default())
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert!(
            rows.iter()
                .find(|m| m.id == id)
                .unwrap()
                .relations
                .is_empty()
        );
    }

    #[tokio::test]
    async fn update_relations() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "with links",
                &V1,
                InsertOptions {
                    relations: &[(10, "blocks".to_owned()), (20, "blocks".to_owned())],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    relations: Some(&[(30, "causes".to_owned())]),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        let mut expected = std::collections::BTreeMap::new();
        expected.insert("30".to_owned(), "causes".to_owned());
        assert_eq!(rows[0].relations, expected);
    }

    #[tokio::test]
    async fn update_relations_to_empty_clears_list() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "with links",
                &V1,
                InsertOptions {
                    relations: &[(5, "blocks".to_owned())],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    relations: Some(&[]),
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert!(rows[0].relations.is_empty());
    }

    #[tokio::test]
    async fn revalidate_batch_empty_returns_zero() {
        let (store, _tmp) = fresh().await;
        let count = store.revalidate_batch(&[], None).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn revalidate_batch_bumps_last_seen_at() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        let before = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let count = store.revalidate_batch(&[id], None).await.unwrap();
        assert_eq!(count, 1);
        let after = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        assert!(
            after > before,
            "last_seen_at should increase after revalidate"
        );
    }

    #[tokio::test]
    async fn revalidate_batch_skips_missing_ids() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        // Mix one real id with one nonexistent id
        let count = store.revalidate_batch(&[id, id + 999], None).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn revalidate_batch_multiple_ids() {
        let (store, _tmp) = fresh().await;
        let id1 = store
            .insert("a", &V1, InsertOptions::default())
            .await
            .unwrap();
        let id2 = store
            .insert("b", &V2, InsertOptions::default())
            .await
            .unwrap();
        let count = store.revalidate_batch(&[id1, id2], None).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn check_embedding_model_records_on_first_open() {
        let (store, _tmp) = fresh().await;
        store
            .check_embedding_model("bge-small-en-v1.5", 384)
            .await
            .unwrap();
        // Second call with the same model must succeed
        store
            .check_embedding_model("bge-small-en-v1.5", 384)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn check_embedding_model_rejects_mismatch() {
        let (store, _tmp) = fresh().await;
        store
            .check_embedding_model("bge-small-en-v1.5", 384)
            .await
            .unwrap();
        let err = store
            .check_embedding_model("bge-large-en-v1.5", 1024)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("mismatch"),
            "error message should mention mismatch: {err}"
        );
    }

    #[tokio::test]
    async fn check_embedding_model_skips_for_in_memory() {
        let store = MemoryStore::open(None).await.unwrap();
        // Different model names must not error for in-memory stores
        store
            .check_embedding_model("bge-small-en-v1.5", 384)
            .await
            .unwrap();
        store
            .check_embedding_model("bge-large-en-v1.5", 1024)
            .await
            .unwrap();
    }

    // Verifying that agent-scoped mutations respect ownership boundaries.
    // A mutation with the wrong agent must not touch rows belonging to another agent.

    #[tokio::test]
    async fn delete_wrong_agent_returns_false() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Attempting to delete alice's memory as bob must fail
        assert!(!store.delete(id, Some("bob")).await.unwrap());
        // Row still exists
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn delete_correct_agent_removes_row() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(store.delete(id, Some("alice")).await.unwrap());
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn update_wrong_agent_returns_false() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "original",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store
            .update(
                id,
                UpdateOptions {
                    content: Some("tampered"),
                    ..Default::default()
                },
                Some("bob"),
            )
            .await
            .unwrap();
        assert!(!updated);
        // Content unchanged
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].content, "original");
    }

    #[tokio::test]
    async fn update_correct_agent_modifies_row() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "original",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = store
            .update(
                id,
                UpdateOptions {
                    content: Some("updated"),
                    ..Default::default()
                },
                Some("alice"),
            )
            .await
            .unwrap();
        assert!(updated);
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].content, "updated");
    }

    #[tokio::test]
    async fn revalidate_wrong_agent_returns_zero() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let before = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let count = store.revalidate_batch(&[id], Some("bob")).await.unwrap();
        assert_eq!(count, 0);
        let after = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        assert_eq!(
            after, before,
            "last_seen_at must not change when agent does not match"
        );
    }

    #[tokio::test]
    async fn revalidate_correct_agent_bumps_clock() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    agent: "alice",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let before = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let count = store.revalidate_batch(&[id], Some("alice")).await.unwrap();
        assert_eq!(count, 1);
        let after = store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].last_seen_at;
        assert!(
            after > before,
            "last_seen_at should increase when agent matches"
        );
    }
}
