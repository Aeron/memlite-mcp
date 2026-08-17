use std::{path::Path, time::Duration};

use anyhow::{Result, anyhow};
use libsql::{Builder, Connection, Value};
use tokio::sync::Mutex;

use super::helpers::{
    build_where, fetch_memories_by_ids, floats_to_blob, parse_memory_row, relations_to_json,
    rerank_by_overlap, tags_to_json, unix_secs,
};
#[cfg(test)]
use super::options::InsertOptions;
use super::options::{FilterOptions, UpdateOptions};
use super::types::{
    BatchInsertItem, GraphStats, HubEntry, Memory, NeighborInfo, Neighbors, PathStep,
    TraverseDirection, TraverseNode, TraverseResult,
};

const BUSY_TIMEOUT: Duration = Duration::from_secs(30);
const VECTOR_INDEX_NAME: &str = "memories_embedding_idx";

async fn validate_relation_targets(tx: &libsql::Transaction, ids: &[i64]) -> Result<()> {
    // Finding missing identifiers with one native JSON table query
    let mut rows = tx
        .query(
            "select coalesce(group_concat(id, ', '), '')
            from (select distinct cast(value as integer) as id
            from json_each(?)
            where not exists (select 1 from memories where rowid = cast(value as integer))
            order by id)",
            libsql::params![serde_json::to_string(ids)?],
        )
        .await?;
    let missing: String = rows
        .next()
        .await?
        .expect("aggregate query returns one row")
        .get(0)?;
    match missing.as_str() {
        "" => Ok(()),
        ids => Err(anyhow!(
            "relations reference missing memory identifiers: {ids}"
        )),
    }
}

/// Provides persistent memory operations over explicit query and mutation connections.
pub struct MemoryStore {
    read_conn: Connection,
    write_conn: Mutex<Connection>,
}

impl MemoryStore {
    /// Opens (or creates) a local SQLite DB at `path`.
    pub async fn open(path: &Path, model_name: &str, embedding_dimensions: usize) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let db = Builder::new_local(path).build().await?;
        let write_conn = db.connect()?;
        write_conn.busy_timeout(BUSY_TIMEOUT)?;
        write_conn.query("pragma journal_mode = wal", ()).await?;
        write_conn
            .execute("pragma synchronous = normal", ())
            .await?;
        super::migrations::run(&write_conn, model_name, embedding_dimensions).await?;

        let read_conn = db.connect()?;
        read_conn.busy_timeout(BUSY_TIMEOUT)?;

        Ok(MemoryStore {
            read_conn,
            write_conn: Mutex::new(write_conn),
        })
    }

    /// Inserts a new memory and returns its rowid.
    ///
    /// `confidence` defaults to 1.0 and `decay_rate` defaults to 0.0 when None.
    /// Both are clamped to [0.0, 1.0].
    #[cfg(test)]
    pub async fn insert(
        &self,
        content: &str,
        embedding: &[f32],
        opts: InsertOptions<'_>,
    ) -> Result<i64> {
        let item = BatchInsertItem {
            content: content.to_owned(),
            embedding: embedding.to_vec(),
            summary: opts.summary.to_owned(),
            tags: opts.tags.to_vec(),
            context: opts.context.map(str::to_owned),
            confidence: opts.confidence.unwrap_or(1.0).clamp(0.0, 1.0),
            decay_rate: opts.decay_rate.unwrap_or(0.0).clamp(0.0, 1.0),
            memory_type: opts.memory_type,
            attrs: opts.attrs.unwrap_or_default(),
            relations: opts.relations.to_vec(),
            refs: opts.refs.to_vec(),
        };
        self.insert_batch(vec![item])
            .await?
            .pop()
            .ok_or_else(|| anyhow!("single-item insert returned no identifier"))
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
        if items.iter().any(|item| item.summary.trim().is_empty()) {
            return Err(anyhow!("summary must not be blank"));
        }

        let conn = self.write_conn.lock().await;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?;
        let relation_ids: Vec<i64> = items
            .iter()
            .flat_map(|item| item.relations.iter().map(|(id, _)| *id))
            .collect();
        validate_relation_targets(&tx, &relation_ids).await?;

        let now = unix_secs();
        let sql = "insert into memories (
                content, summary, tags, embedding, created_at, updated_at, context,
                confidence, decay_rate, last_seen_at, type, relations, refs, attrs
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
        let mut ids = Vec::with_capacity(items.len());

        for item in items {
            let tags_json = tags_to_json(&item.tags)?;
            let relations_json = relations_to_json(&item.relations)?;
            let item_refs_json = serde_json::to_string(&item.refs)?;
            let item_attrs_json = serde_json::to_string(&item.attrs)?;
            let emb_blob = floats_to_blob(&item.embedding);

            tx.execute(
                sql,
                libsql::params_from_iter([
                    Value::Text(item.content),
                    Value::Text(item.summary),
                    Value::Text(tags_json.clone()),
                    Value::Blob(emb_blob),
                    Value::Integer(now),
                    Value::Integer(now),
                    item.context.map(Value::Text).unwrap_or(Value::Null),
                    Value::Real(item.confidence),
                    Value::Real(item.decay_rate),
                    Value::Integer(now),
                    Value::Text(item.memory_type.to_string()),
                    Value::Text(relations_json.clone()),
                    Value::Text(item_refs_json),
                    Value::Text(item_attrs_json),
                ]),
            )
            .await?;

            let id = tx.last_insert_rowid();
            ids.push(id);

            // Populating normalized tables from the same canonical JSON values
            tx.execute(
                "insert or ignore into memory_tags (memory_id, tag)
                 select ?, value from json_each(?) where trim(value) != ''",
                libsql::params![id, tags_json],
            )
            .await?;
            tx.execute(
                "insert or ignore into memory_relations (from_id, to_id, kind)
                 select ?, cast(key as integer), value from json_each(?)",
                libsql::params![id, relations_json],
            )
            .await?;
        }

        tx.commit().await?;

        Ok(ids)
    }

    /// Returns memories ordered by cosine distance to `query_embedding`.
    ///
    /// Fetches up to `limit * 4` candidates from the database (capped at 400),
    /// then re-ranks by word-overlap between `query_text` and each memory's content.
    /// Unfiltered searches use the vector index; filtered searches retain exact
    /// pre-filter semantics with a cosine scan. Returns the top `limit` results.
    pub async fn search(
        &self,
        query_text: &str,
        query_embedding: &[f32],
        filter: FilterOptions<'_>,
        limit: u32,
    ) -> Result<Vec<(Memory, f64)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let emb_blob = floats_to_blob(query_embedding);
        let candidate_limit = (limit as i64).saturating_mul(4).min(400);
        let (sql, params) = if filter.is_empty() {
            (
                format!(
                    "select
                        m.rowid, m.content, m.summary, m.tags,
                        m.created_at, m.updated_at, m.context,
                        m.confidence, m.decay_rate, m.last_seen_at, m.type,
                        m.relations, m.refs, m.attrs,
                        vector_distance_cos(m.embedding, ?1) as dist
                    from vector_top_k('{VECTOR_INDEX_NAME}', ?1, ?2) as v
                    join memories m on m.rowid = v.id
                    order by dist asc"
                ),
                vec![Value::Blob(emb_blob), Value::Integer(candidate_limit)],
            )
        } else {
            let mut params = vec![Value::Blob(emb_blob)];
            let where_sql = build_where(&filter, &mut params)?;
            params.push(Value::Integer(candidate_limit));
            (
                format!(
                    "select
                        rowid, content, summary, tags,
                        created_at, updated_at, context,
                        confidence, decay_rate, last_seen_at, type, relations, refs, attrs,
                        vector_distance_cos(embedding, ?) as dist
                    from memories
                    {where_sql}
                    order by dist asc
                    limit ?"
                ),
                params,
            )
        };
        let mut rows = self
            .read_conn
            .query(&sql, libsql::params_from_iter(params))
            .await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            let memory = parse_memory_row(&row)?;
            let dist: f64 = row.get(14)?;
            results.push((memory, dist));
        }

        Ok(rerank_by_overlap(query_text, results, limit as usize))
    }

    /// Returns memories ordered by creation time, newest first.
    ///
    /// Optionally pre-filters by identifiers, tags, type, attrs, or refs.
    pub async fn list(
        &self,
        filter: FilterOptions<'_>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Memory>> {
        let mut params: Vec<Value> = Vec::new();
        let where_sql = build_where(&filter, &mut params)?;
        params.push(Value::Integer(limit as i64));
        params.push(Value::Integer(offset as i64));

        let sql = format!(
            "select
                rowid, content, summary, tags, created_at, updated_at, context,
                confidence, decay_rate, last_seen_at, type, relations, refs, attrs
            from memories
            {where_sql}
            order by created_at desc
            limit ? offset ?"
        );
        let mut rows = self
            .read_conn
            .query(&sql, libsql::params_from_iter(params))
            .await?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            results.push(parse_memory_row(&row)?);
        }

        Ok(results)
    }

    /// Returns all distinct tags with occurrence counts, ordered by count descending.
    ///
    /// Reads from the normalized `memory_tags` table (indexed).
    pub async fn tags(&self) -> Result<Vec<(String, u32)>> {
        let mut rows = self
            .read_conn
            .query(
                "select tag, count(*) as cnt
                from memory_tags
                group by tag
                order by cnt desc",
                (),
            )
            .await?;
        let mut results = Vec::new();
        while let Some(row) = rows.next().await? {
            let tag: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            results.push((tag, count as u32));
        }
        Ok(results)
    }

    /// Returns whether a memory with the given row identifier exists.
    pub async fn contains(&self, id: i64) -> Result<bool> {
        let mut rows = self
            .read_conn
            .query(
                "select 1 from memories where rowid = ? limit 1",
                libsql::params![id],
            )
            .await?;
        Ok(rows.next().await?.is_some())
    }

    /// Deletes a memory by rowid; returns true if a row was actually removed.
    pub async fn delete(&self, id: i64) -> Result<bool> {
        let conn = self.write_conn.lock().await;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?;
        let affected = tx
            .execute("delete from memories where rowid = ?", libsql::params![id])
            .await?;

        // Cleaning up auxiliary rows for the deleted memory
        if affected > 0 {
            tx.execute(
                "delete from memory_tags where memory_id = ?",
                libsql::params![id],
            )
            .await?;

            // Removing incoming links from source JSON before deleting normalized edges
            let relation_path = format!("$.\"{id}\"");
            tx.execute(
                "update memories
                 set relations = json_remove(relations, ?), updated_at = ?
                 where rowid in (
                    select from_id from memory_relations where to_id = ?
                 )",
                libsql::params![relation_path, unix_secs(), id],
            )
            .await?;
            tx.execute(
                "delete from memory_relations where from_id = ? or to_id = ?",
                libsql::params![id, id],
            )
            .await?;
        }

        tx.commit().await?;

        Ok(affected > 0)
    }

    /// Updates the specified fields of a memory row.
    ///
    /// - Blank `summary` values are rejected.
    /// - `context = Some("")` clears the context (sets to NULL).
    /// - `attrs = Some({})` clears all type-specific extra fields.
    /// - Supplying a new `embedding` alongside updated content keeps the index
    ///   consistent; pass None to leave the stored vector unchanged.
    /// - `confidence` and `decay_rate` are clamped to [0.0, 1.0] when provided.
    /// - `last_seen_at` is always bumped to now when any field is updated,
    ///   resetting the confidence decay clock.
    /// - Returns true when at least one row was modified.
    pub async fn update(&self, id: i64, opts: UpdateOptions<'_>) -> Result<bool> {
        let UpdateOptions {
            content,
            summary,
            tags,
            embedding,
            context,
            confidence,
            decay_rate,
            memory_type,
            attrs,
            relations,
            refs,
        } = opts;
        if summary.is_some_and(|value| value.trim().is_empty()) {
            return Err(anyhow!("summary must not be blank"));
        }
        if content.is_none()
            && summary.is_none()
            && tags.is_none()
            && embedding.is_none()
            && context.is_none()
            && confidence.is_none()
            && decay_rate.is_none()
            && memory_type.is_none()
            && attrs.is_none()
            && relations.is_none()
            && refs.is_none()
        {
            return Ok(false);
        }

        let tags_json = tags.map(tags_to_json).transpose()?;
        let relations_json = relations.map(relations_to_json).transpose()?;
        let conn = self.write_conn.lock().await;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await?;
        if let Some(updated_relations) = relations {
            let relation_ids: Vec<i64> = updated_relations
                .iter()
                .map(|(target_id, _)| *target_id)
                .collect();
            validate_relation_targets(&tx, &relation_ids).await?;
        }
        let now = unix_secs();

        // Building SET clauses and positional params for each provided field
        let mut set_clauses: Vec<&str> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        if let Some(c) = content {
            set_clauses.push("content = ?");
            params.push(Value::Text(c.to_owned()));
        }
        if let Some(s) = summary {
            set_clauses.push("summary = ?");
            params.push(Value::Text(s.to_owned()));
        }
        if let Some(value) = &tags_json {
            set_clauses.push("tags = ?");
            params.push(Value::Text(value.clone()));
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
            params.push(Value::Text(mt.to_string()));
        }
        if let Some(a) = &attrs {
            set_clauses.push("attrs = ?");
            params.push(Value::Text(serde_json::to_string(a)?));
        }
        if let Some(value) = &relations_json {
            set_clauses.push("relations = ?");
            params.push(Value::Text(value.clone()));
        }
        if let Some(r) = refs {
            set_clauses.push("refs = ?");
            params.push(Value::Text(serde_json::to_string(r)?));
        }
        // Always bump both timestamps: updated_at records the data change,
        // last_seen_at resets the confidence decay clock
        set_clauses.push("updated_at = ?");
        params.push(Value::Integer(now));
        set_clauses.push("last_seen_at = ?");
        params.push(Value::Integer(now));
        params.push(Value::Integer(id));
        let sql = format!(
            "update memories set {} where rowid = ?",
            set_clauses.join(", ")
        );

        let affected = tx.execute(&sql, libsql::params_from_iter(params)).await?;

        // Synchronizing normalized tables from the same canonical JSON values
        if affected > 0
            && let Some(value) = tags_json
        {
            tx.execute(
                "delete from memory_tags where memory_id = ?",
                libsql::params![id],
            )
            .await?;
            tx.execute(
                "insert or ignore into memory_tags (memory_id, tag)
                 select ?, value from json_each(?) where trim(value) != ''",
                libsql::params![id, value],
            )
            .await?;
        }

        if affected > 0
            && let Some(value) = relations_json
        {
            tx.execute(
                "delete from memory_relations where from_id = ?",
                libsql::params![id],
            )
            .await?;
            tx.execute(
                "insert or ignore into memory_relations (from_id, to_id, kind)
                 select ?, cast(key as integer), value from json_each(?)",
                libsql::params![id, value],
            )
            .await?;
        }

        tx.commit().await?;

        Ok(affected > 0)
    }

    /// Resets the decay clock on a batch of memories without changing any other field.
    ///
    /// Issues a single update touching only `last_seen_at`. Returns the count of rows
    /// actually modified; may be less than `ids.len()` when some identifiers do not exist.
    pub async fn revalidate_batch(&self, ids: &[i64]) -> Result<u64> {
        if ids.is_empty() {
            return Ok(0);
        }
        let conn = self.write_conn.lock().await;
        let now = unix_secs();
        let placeholders = vec!["?"; ids.len()].join(", ");
        let sql = format!("update memories set last_seen_at = ? where rowid in ({placeholders})");
        let mut params: Vec<Value> = Vec::with_capacity(1 + ids.len());
        params.push(Value::Integer(now));
        for id in ids {
            params.push(Value::Integer(*id));
        }
        let affected = conn.execute(&sql, libsql::params_from_iter(params)).await?;
        Ok(affected)
    }

    /// Returns the immediate neighbors of a memory node, with optional kind filtering.
    ///
    /// Both outgoing (this node -> others) and incoming (others -> this node) edges are
    /// returned. When `kind_filter` is supplied, only edges with that exact kind are included.
    pub async fn neighbors(&self, id: i64, kind_filter: Option<&str>) -> Result<Neighbors> {
        let conn = &self.read_conn;

        // Outgoing: this memory -> others
        let (out_sql, out_params): (&str, Vec<Value>) = if let Some(kf) = kind_filter {
            (
                "select to_id, kind from memory_relations where from_id = ? and kind = ? order by kind",
                vec![Value::Integer(id), Value::Text(kf.to_owned())],
            )
        } else {
            (
                "select to_id, kind from memory_relations where from_id = ? order by kind",
                vec![Value::Integer(id)],
            )
        };
        let mut out_pairs: Vec<(i64, String)> = Vec::new();

        let mut rows = conn
            .query(out_sql, libsql::params_from_iter(out_params))
            .await?;
        while let Some(row) = rows.next().await? {
            out_pairs.push((row.get(0)?, row.get(1)?));
        }

        // Incoming: others -> this memory
        let (in_sql, in_params): (&str, Vec<Value>) = if let Some(kf) = kind_filter {
            (
                "select from_id, kind from memory_relations where to_id = ? and kind = ? order by kind",
                vec![Value::Integer(id), Value::Text(kf.to_owned())],
            )
        } else {
            (
                "select from_id, kind from memory_relations where to_id = ? order by kind",
                vec![Value::Integer(id)],
            )
        };
        let mut in_pairs: Vec<(i64, String)> = Vec::new();

        let mut rows = conn
            .query(in_sql, libsql::params_from_iter(in_params))
            .await?;
        while let Some(row) = rows.next().await? {
            in_pairs.push((row.get(0)?, row.get(1)?));
        }

        // Fetch full memories for all referenced ids in a single query
        let unique_other_ids: std::collections::HashSet<i64> = out_pairs
            .iter()
            .map(|(oid, _)| *oid)
            .chain(in_pairs.iter().map(|(oid, _)| *oid))
            .collect();
        let all_other_ids: Vec<i64> = unique_other_ids.into_iter().collect();
        let memory_map = fetch_memories_by_ids(conn, &all_other_ids).await?;

        let outgoing = out_pairs
            .into_iter()
            .map(|(other_id, kind)| NeighborInfo {
                memory: memory_map.get(&other_id).cloned(),
                other_id,
                kind,
            })
            .collect();
        let incoming = in_pairs
            .into_iter()
            .map(|(other_id, kind)| NeighborInfo {
                memory: memory_map.get(&other_id).cloned(),
                other_id,
                kind,
            })
            .collect();

        Ok(Neighbors { outgoing, incoming })
    }

    /// Runs a BFS from `start_id`, following edges up to `max_depth` hops.
    ///
    /// `direction` controls which edges are followed: outgoing, incoming, or both.
    /// `kind_filter` restricts traversal to edges with that exact kind label.
    /// Depth is capped at 5 regardless of the supplied value.
    pub async fn traverse(
        &self,
        start_id: i64,
        max_depth: u32,
        kind_filter: Option<&str>,
        direction: TraverseDirection,
    ) -> Result<TraverseResult> {
        let conn = &self.read_conn;
        let max_depth = max_depth.min(5);

        // visited: id -> (depth, via_kind used to first reach this node)
        let mut visited: std::collections::HashMap<i64, (u32, Option<String>)> =
            std::collections::HashMap::new();
        visited.insert(start_id, (0, None));
        // edges: (from_id, to_id, kind), deduplicated at the end
        let mut edges: std::collections::HashSet<(i64, i64, String)> =
            std::collections::HashSet::new();
        let mut frontier: Vec<i64> = vec![start_id];

        for depth in 1..=max_depth {
            if frontier.is_empty() {
                break;
            }
            let placeholders = vec!["?"; frontier.len()].join(", ");
            // next_nodes: (new_id, via_kind); may contain duplicates, deduped via visited check
            let mut next_nodes: Vec<(i64, String)> = Vec::new();

            // Outgoing edges from frontier nodes
            if matches!(direction, TraverseDirection::Out | TraverseDirection::Both) {
                let sql = if kind_filter.is_some() {
                    format!(
                        "select from_id, to_id, kind from memory_relations where from_id in ({placeholders}) and kind = ?"
                    )
                } else {
                    format!(
                        "select from_id, to_id, kind from memory_relations where from_id in ({placeholders})"
                    )
                };
                let mut params: Vec<Value> =
                    frontier.iter().map(|&fid| Value::Integer(fid)).collect();
                if let Some(kf) = kind_filter {
                    params.push(Value::Text(kf.to_owned()));
                }

                let mut rows = conn.query(&sql, libsql::params_from_iter(params)).await?;
                while let Some(row) = rows.next().await? {
                    let from_id: i64 = row.get(0)?;
                    let to_id: i64 = row.get(1)?;
                    let kind: String = row.get(2)?;
                    if !visited.contains_key(&to_id) {
                        next_nodes.push((to_id, kind.clone()));
                    }
                    edges.insert((from_id, to_id, kind));
                }
            }

            // Incoming edges to frontier nodes
            if matches!(direction, TraverseDirection::In | TraverseDirection::Both) {
                let sql = if kind_filter.is_some() {
                    format!(
                        "select from_id, to_id, kind from memory_relations where to_id in ({placeholders}) and kind = ?"
                    )
                } else {
                    format!(
                        "select from_id, to_id, kind from memory_relations where to_id in ({placeholders})"
                    )
                };
                let mut params: Vec<Value> =
                    frontier.iter().map(|&fid| Value::Integer(fid)).collect();
                if let Some(kf) = kind_filter {
                    params.push(Value::Text(kf.to_owned()));
                }

                let mut rows = conn.query(&sql, libsql::params_from_iter(params)).await?;
                while let Some(row) = rows.next().await? {
                    let from_id: i64 = row.get(0)?;
                    let to_id: i64 = row.get(1)?;
                    let kind: String = row.get(2)?;

                    // New node when traversing backwards is the from_id
                    if !visited.contains_key(&from_id) {
                        next_nodes.push((from_id, kind.clone()));
                    }

                    edges.insert((from_id, to_id, kind));
                }
            }

            // Dedup and add to frontier
            let mut new_frontier: Vec<i64> = Vec::new();
            let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
            for (new_id, via_kind) in next_nodes {
                if !visited.contains_key(&new_id) && seen.insert(new_id) {
                    visited.insert(new_id, (depth, Some(via_kind)));
                    new_frontier.push(new_id);
                }
            }
            frontier = new_frontier;
        }

        // Fetch memories for all discovered nodes
        let all_ids: Vec<i64> = visited.keys().copied().collect();
        let memory_map = fetch_memories_by_ids(conn, &all_ids).await?;

        let mut nodes: Vec<TraverseNode> = visited
            .into_iter()
            .filter_map(|(nid, (depth, via_kind))| {
                memory_map.get(&nid).map(|m| TraverseNode {
                    memory: m.clone(),
                    depth,
                    via_kind,
                })
            })
            .collect();
        nodes.sort_by_key(|n| (n.depth, n.memory.id));

        let mut edge_vec: Vec<(i64, i64, String)> = edges.into_iter().collect();
        edge_vec.sort();

        Ok(TraverseResult {
            start_id,
            nodes,
            edges: edge_vec,
        })
    }

    /// Finds the shortest directed path from `from_id` to `to_id` via outgoing edges.
    ///
    /// Returns None when no path exists within `max_hops` (capped at 10).
    /// The returned vec starts with the source memory (via_kind = None) and ends
    /// with the destination, each step carrying the relation kind used to arrive.
    pub async fn path(
        &self,
        from_id: i64,
        to_id: i64,
        max_hops: u32,
    ) -> Result<Option<Vec<PathStep>>> {
        if from_id == to_id {
            let map = fetch_memories_by_ids(&self.read_conn, &[from_id]).await?;
            return Ok(map.get(&from_id).map(|m| {
                vec![PathStep {
                    memory: m.clone(),
                    via_kind: None,
                }]
            }));
        }

        let conn = &self.read_conn;
        let max_hops = max_hops.min(10);

        // BFS with parent tracking: child_id -> (parent_id, edge_kind)
        let mut parent: std::collections::HashMap<i64, (i64, String)> =
            std::collections::HashMap::new();
        let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
        visited.insert(from_id);
        let mut frontier: Vec<i64> = vec![from_id];
        let mut found = false;

        'bfs: for _ in 0..max_hops {
            if frontier.is_empty() {
                break;
            }
            let placeholders = vec!["?"; frontier.len()].join(", ");
            let sql = format!(
                "select from_id, to_id, kind from memory_relations where from_id in ({placeholders})"
            );
            let params: Vec<Value> = frontier.iter().map(|&fid| Value::Integer(fid)).collect();
            let mut rows = conn.query(&sql, libsql::params_from_iter(params)).await?;
            let mut next_frontier: Vec<i64> = Vec::new();
            while let Some(row) = rows.next().await? {
                let f: i64 = row.get(0)?;
                let t: i64 = row.get(1)?;
                let kind: String = row.get(2)?;
                if !visited.contains(&t) {
                    visited.insert(t);
                    parent.insert(t, (f, kind));
                    if t == to_id {
                        found = true;
                        break 'bfs;
                    }
                    next_frontier.push(t);
                }
            }
            frontier = next_frontier;
        }

        if !found {
            return Ok(None);
        }

        // Reconstruct path by walking parent links from to_id back to from_id
        let mut path_ids: Vec<(i64, Option<String>)> = Vec::new();
        let mut current = to_id;
        loop {
            if current == from_id {
                path_ids.push((from_id, None));
                break;
            }
            match parent.get(&current) {
                Some((p, kind)) => {
                    path_ids.push((current, Some(kind.clone())));
                    current = *p;
                }
                None => {
                    path_ids.push((current, None));
                    break;
                }
            }
        }
        path_ids.reverse();

        // Fetch memories for all path nodes
        let ids: Vec<i64> = path_ids.iter().map(|(id, _)| *id).collect();
        let memory_map = fetch_memories_by_ids(conn, &ids).await?;

        let steps = path_ids
            .into_iter()
            .filter_map(|(id, via_kind)| {
                memory_map.get(&id).map(|m| PathStep {
                    memory: m.clone(),
                    via_kind,
                })
            })
            .collect();

        Ok(Some(steps))
    }

    /// Returns the top-N most-connected memories by total degree (out + in).
    ///
    /// Degree is computed globally across all agents. Memories without any relations
    /// are not included. The result is ordered by total degree descending.
    pub async fn hubs(&self, top_n: u32) -> Result<Vec<HubEntry>> {
        let top_n = top_n.min(50) as i64;

        // Using a CTE to union out-degree and in-degree aggregates, then join with memories
        let sql = "
            with out_deg as (
                select from_id as id, count(*) as cnt
                from memory_relations
                group by from_id
            ),
            in_deg as (
                select to_id as id, count(*) as cnt
                from memory_relations
                group by to_id
            ),
            all_ids as (
                select id from out_deg
                union
                select id from in_deg
            )
            select
                a.id,
                coalesce(o.cnt, 0) as out_degree,
                coalesce(i.cnt, 0) as in_degree,
                m.summary,
                m.type
            from all_ids a
            left join out_deg o on o.id = a.id
            left join in_deg i on i.id = a.id
            join memories m on m.rowid = a.id
            order by (coalesce(o.cnt, 0) + coalesce(i.cnt, 0)) desc, a.id asc
            limit ?
        ";

        let mut rows = self.read_conn.query(sql, libsql::params![top_n]).await?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().await? {
            let id: i64 = row.get(0)?;
            let out_degree: i64 = row.get(1)?;
            let in_degree: i64 = row.get(2)?;
            let summary: String = row.get(3)?;
            let memory_type = match row.get_value(4)? {
                Value::Null => None,
                Value::Text(s) => s.parse().ok(),
                _ => None,
            };
            entries.push(HubEntry {
                id,
                summary,
                memory_type,
                out_degree: out_degree as u32,
                in_degree: in_degree as u32,
            });
        }
        Ok(entries)
    }

    /// Returns graph-level statistics: memory counts, relation totals, and relation kinds.
    ///
    /// All counts are global across agents, matching the rest of the graph subsystem
    /// (`neighbors`, `traverse`, `path`, `hubs`): the relation graph is a single shared
    /// structure with no agent column, and edges may legitimately cross agent boundaries.
    pub async fn graph_stats(&self) -> Result<GraphStats> {
        let conn = &self.read_conn;

        // Total memories
        let mut rows = conn.query("select count(*) from memories", ()).await?;
        let total_memories = rows
            .next()
            .await?
            .map(|r| r.get::<i64>(0).unwrap_or(0) as u64)
            .unwrap_or(0);

        // Memories by type
        let mut rows = conn
            .query(
                "select type, count(*) from memories group by type order by count(*) desc",
                (),
            )
            .await?;
        let mut by_type: Vec<(Option<String>, u64)> = Vec::new();
        while let Some(row) = rows.next().await? {
            let type_name = match row.get_value(0)? {
                Value::Null => None,
                Value::Text(s) => Some(s),
                _ => None,
            };
            let cnt: i64 = row.get(1)?;
            by_type.push((type_name, cnt as u64));
        }

        // Total relation edges (always global)
        let mut rows = conn
            .query("select count(*) from memory_relations", ())
            .await?;
        let total_relations = rows
            .next()
            .await?
            .map(|r| r.get::<i64>(0).unwrap_or(0) as u64)
            .unwrap_or(0);

        // Distinct relation kinds
        let mut rows = conn
            .query(
                "select distinct kind from memory_relations order by kind",
                (),
            )
            .await?;
        let mut relation_kinds: Vec<String> = Vec::new();
        while let Some(row) = rows.next().await? {
            relation_kinds.push(row.get(0)?);
        }

        Ok(GraphStats {
            total_memories,
            by_type,
            total_relations,
            relation_kinds,
        })
    }
}

#[cfg(test)]
mod tests {
    use tempfile::NamedTempFile;

    use super::*;
    use crate::db::{MemoryAttrSeverity, MemoryAttrs, MemoryStatus, MemoryType};

    async fn fresh() -> (MemoryStore, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = MemoryStore::open(tmp.path(), "test-model", 2)
            .await
            .unwrap();
        (store, tmp)
    }

    #[tokio::test]
    async fn read_connection_excludes_uncommitted_writes() {
        let (store, _tmp) = fresh().await;
        let conn = store.write_conn.lock().await;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .unwrap();
        tx.execute(
            "insert into meta (key, value) values (?, ?)",
            libsql::params!["test", "pending"],
        )
        .await
        .unwrap();

        let mut rows = store
            .read_conn
            .query("select value from meta where key = 'test'", ())
            .await
            .unwrap();
        assert!(rows.next().await.unwrap().is_none());
        tx.rollback().await.unwrap();
    }

    // 2-D unit vectors used throughout: cheap, no model download needed
    const V1: [f32; 2] = [1.0, 0.0];
    const V2: [f32; 2] = [0.0, 1.0];

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
                    summary: "summ",
                    tags: &["tag".into()],
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
        assert_eq!(m.summary, "summ");
        assert_eq!(m.tags, ["tag"]);
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
    async fn list_filters_by_ids() {
        let (store, _tmp) = fresh().await;
        let tagged_id = store
            .insert(
                "tagged",
                &V1,
                InsertOptions {
                    tags: &["bug".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let other_id = store
            .insert("other", &V1, InsertOptions::default())
            .await
            .unwrap();

        let exact = store
            .list(
                FilterOptions {
                    ids: Some(&[tagged_id, 999]),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].id, tagged_id);

        let composed = store
            .list(
                FilterOptions {
                    ids: Some(&[tagged_id, other_id]),
                    tags: Some(&["bug".into()]),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert_eq!(composed.len(), 1);
        assert_eq!(composed[0].id, tagged_id);

        let empty = store
            .list(
                FilterOptions {
                    ids: Some(&[]),
                    ..Default::default()
                },
                10,
                0,
            )
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    #[tokio::test]
    async fn delete_existing_returns_true() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        assert!(store.delete(id).await.unwrap());
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
        assert!(!store.delete(999).await.unwrap());
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

        let counts = store.tags().await.unwrap();
        assert_eq!(counts.len(), 2);
        // ordered by count desc: "a" (2) before "b" (1)
        assert_eq!(counts[0], ("a".into(), 2));
        assert_eq!(counts[1], ("b".into(), 1));
    }

    #[tokio::test]
    async fn insert_and_update_drop_blank_tags_consistently() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "tagged",
                &V1,
                InsertOptions {
                    tags: &["rust".into(), String::new(), "  ".into()],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].tags,
            ["rust"]
        );

        store
            .update(
                id,
                UpdateOptions {
                    tags: Some(&[String::new(), "go".into()]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].tags,
            ["go"]
        );
        assert_eq!(store.tags().await.unwrap(), [("go".into(), 1)]);
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
    async fn update_rejects_blank_summary() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "x",
                &V1,
                InsertOptions {
                    summary: "summ",
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let error = store
            .update(
                id,
                UpdateOptions {
                    summary: Some(""),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "summary must not be blank");
        assert_eq!(
            store.list(FilterOptions::default(), 1, 0).await.unwrap()[0].summary,
            "summ"
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
        assert!(!store.update(id, UpdateOptions::default()).await.unwrap());
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
        let conn = store.write_conn.lock().await;
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
    async fn list_filters_by_memory_type() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "architectural choice",
                &V1,
                InsertOptions {
                    memory_type: MemoryType::Decision,
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
                    memory_type: MemoryType::Issue,
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
                    memory_type: MemoryType::Reference,
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
                    attrs: Some(MemoryAttrs {
                        status: Some(MemoryStatus::Open),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].attrs.status, Some(MemoryStatus::Open));
    }

    #[tokio::test]
    async fn list_filters_by_status() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "open issue",
                &V1,
                InsertOptions {
                    attrs: Some(MemoryAttrs {
                        status: Some(MemoryStatus::Open),
                        ..Default::default()
                    }),
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
                    attrs: Some(MemoryAttrs {
                        status: Some(MemoryStatus::Resolved),
                        ..Default::default()
                    }),
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
                    attrs: Some(MemoryAttrs {
                        status: Some(MemoryStatus::Open),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let updated = store
            .update(
                id,
                UpdateOptions {
                    attrs: Some(MemoryAttrs {
                        status: Some(MemoryStatus::Resolved),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(updated);

        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].attrs.status, Some(MemoryStatus::Resolved));
    }

    #[tokio::test]
    async fn insert_batch_empty_returns_empty() {
        let (store, _tmp) = fresh().await;
        let ids = store.insert_batch(vec![]).await.unwrap();
        assert!(ids.is_empty());
    }

    #[tokio::test]
    async fn insert_batch_rejects_blank_summary() {
        let (store, _tmp) = fresh().await;
        let error = store
            .insert_batch(vec![BatchInsertItem {
                content: "content".into(),
                embedding: V1.to_vec(),
                summary: "  ".into(),
                tags: vec![],
                context: None,
                confidence: 1.0,
                decay_rate: 0.0,
                memory_type: MemoryType::Project,
                attrs: MemoryAttrs::default(),
                relations: vec![],
                refs: vec![],
            }])
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "summary must not be blank");
        assert!(
            store
                .list(FilterOptions::default(), 1, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn insert_batch_returns_ids_in_order() {
        let (store, _tmp) = fresh().await;
        let items = vec![
            BatchInsertItem {
                content: "first".into(),
                embedding: V1.to_vec(),
                summary: "s1".into(),
                tags: vec!["a".into()],
                context: None,
                confidence: 1.0,
                decay_rate: 0.0,
                memory_type: MemoryType::Project,
                attrs: MemoryAttrs {
                    status: Some(MemoryStatus::Open),
                    ..Default::default()
                },
                relations: vec![],
                refs: vec![],
            },
            BatchInsertItem {
                content: "second".into(),
                embedding: V2.to_vec(),
                summary: "s2".into(),
                tags: vec![],
                context: None,
                confidence: 0.8,
                decay_rate: 0.01,
                memory_type: MemoryType::Task,
                attrs: MemoryAttrs {
                    status: Some(MemoryStatus::Resolved),
                    ..Default::default()
                },
                relations: vec![],
                refs: vec![],
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
        assert_eq!(first.summary, "s1");
        assert_eq!(first.tags, ["a"]);
        assert_eq!(first.memory_type, Some(MemoryType::Project));
        assert_eq!(first.attrs.status, Some(MemoryStatus::Open));
        assert_eq!(second.attrs.status, Some(MemoryStatus::Resolved));
    }

    #[tokio::test]
    async fn insert_and_list_roundtrip_with_severity() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert(
                "sql injection in login",
                &V1,
                InsertOptions {
                    attrs: Some(MemoryAttrs {
                        severity: Some(MemoryAttrSeverity::Critical),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 10, 0).await.unwrap();
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].attrs.severity, Some(MemoryAttrSeverity::Critical));
    }

    #[tokio::test]
    async fn list_filters_by_severity() {
        let (store, _tmp) = fresh().await;
        store
            .insert(
                "critical bug",
                &V1,
                InsertOptions {
                    attrs: Some(MemoryAttrs {
                        severity: Some(MemoryAttrSeverity::Critical),
                        ..Default::default()
                    }),
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
                    attrs: Some(MemoryAttrs {
                        severity: Some(MemoryAttrSeverity::Low),
                        ..Default::default()
                    }),
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
                    severity: Some(MemoryAttrSeverity::Critical),
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
                    attrs: Some(MemoryAttrs {
                        severity: Some(MemoryAttrSeverity::Medium),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    attrs: Some(MemoryAttrs {
                        severity: Some(MemoryAttrSeverity::High),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert_eq!(rows[0].attrs.severity, Some(MemoryAttrSeverity::High));
    }

    #[tokio::test]
    async fn insert_and_list_roundtrip_with_relations() {
        let (store, _tmp) = fresh().await;
        // Inserting targets before creating links
        let target_id = store
            .insert("target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let other_target_id = store
            .insert("other target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let id = store
            .insert(
                "source with links",
                &V1,
                InsertOptions {
                    relations: &[
                        (target_id, "caused_by".to_owned()),
                        (other_target_id, "related_to".to_owned()),
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
        expected.insert(other_target_id.to_string(), "related_to".to_owned());
        assert_eq!(source.relations, expected);
    }

    #[tokio::test]
    async fn insert_rejects_missing_relation_target_atomically() {
        let (store, _tmp) = fresh().await;
        let error = store
            .insert(
                "invalid source",
                &V1,
                InsertOptions {
                    relations: &[(999, "related_to".to_owned())],
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("missing memory identifiers"));
        assert!(
            store
                .list(FilterOptions::default(), 10, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn delete_target_removes_incoming_relation_from_both_stores() {
        let (store, _tmp) = fresh().await;
        let target_id = store
            .insert("target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let source_id = store
            .insert(
                "source",
                &V1,
                InsertOptions {
                    relations: &[(target_id, "related_to".to_owned())],
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(store.delete(target_id).await.unwrap());
        let source_ids = [source_id];
        let memories = store
            .list(
                FilterOptions {
                    ids: Some(&source_ids),
                    ..Default::default()
                },
                1,
                0,
            )
            .await
            .unwrap();
        assert!(memories[0].relations.is_empty());
        assert!(
            store
                .neighbors(source_id, None)
                .await
                .unwrap()
                .outgoing
                .is_empty()
        );
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
        let first_target = store
            .insert("first target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let second_target = store
            .insert("second target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let replacement_target = store
            .insert("replacement target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let id = store
            .insert(
                "with links",
                &V1,
                InsertOptions {
                    relations: &[
                        (first_target, "blocks".to_owned()),
                        (second_target, "blocks".to_owned()),
                    ],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        store
            .update(
                id,
                UpdateOptions {
                    relations: Some(&[(replacement_target, "causes".to_owned())]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let ids = [id];
        let rows = store
            .list(
                FilterOptions {
                    ids: Some(&ids),
                    ..Default::default()
                },
                1,
                0,
            )
            .await
            .unwrap();
        let expected = std::collections::BTreeMap::from([(
            replacement_target.to_string(),
            "causes".to_owned(),
        )]);
        assert_eq!(rows[0].relations, expected);
    }

    #[tokio::test]
    async fn update_relations_to_empty_clears_list() {
        let (store, _tmp) = fresh().await;
        let target_id = store
            .insert("target", &V2, InsertOptions::default())
            .await
            .unwrap();
        let id = store
            .insert(
                "with links",
                &V1,
                InsertOptions {
                    relations: &[(target_id, "blocks".to_owned())],
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
            )
            .await
            .unwrap();
        let rows = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert!(rows[0].relations.is_empty());
    }

    #[tokio::test]
    async fn revalidate_batch_empty_returns_zero() {
        let (store, _tmp) = fresh().await;
        let count = store.revalidate_batch(&[]).await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn revalidate_batch_bumps_last_seen_at() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        let before = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        let before_last_seen = before[0].last_seen_at;
        let before_updated = before[0].updated_at;
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let count = store.revalidate_batch(&[id]).await.unwrap();
        assert_eq!(count, 1);
        let after = store.list(FilterOptions::default(), 1, 0).await.unwrap();
        assert!(
            after[0].last_seen_at > before_last_seen,
            "last_seen_at should increase after revalidate"
        );
        assert_eq!(after[0].updated_at, before_updated);
    }

    #[tokio::test]
    async fn revalidate_batch_skips_missing_ids() {
        let (store, _tmp) = fresh().await;
        let id = store
            .insert("x", &V1, InsertOptions::default())
            .await
            .unwrap();
        // Mix one real id with one nonexistent id
        let count = store.revalidate_batch(&[id, id + 999]).await.unwrap();
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
        let count = store.revalidate_batch(&[id1, id2]).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn open_rejects_embedding_model_mismatch() {
        let tmp = NamedTempFile::new().unwrap();
        MemoryStore::open(tmp.path(), "first-model", 2)
            .await
            .unwrap();
        let error = MemoryStore::open(tmp.path(), "second-model", 2)
            .await
            .err()
            .expect("mismatched model should fail");
        assert!(
            error.to_string().contains("mismatch"),
            "error message should mention mismatch: {error}"
        );
    }
}
