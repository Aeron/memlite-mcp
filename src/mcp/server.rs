use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use anyhow::anyhow;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    serde_json,
    service::{NotificationContext, QuitReason},
    tool, tool_handler, tool_router,
};
use tokio::sync::{Notify, OnceCell};

use super::params::{
    BatchStoreParams, DeleteParams, GraphStatsParams, HubsParams, ListParams, NeighborsParams,
    PathParams, RevalidateParams, SearchParams, StoreParams, TagsParams, TraverseParams,
    UpdateParams,
};
use crate::{
    config::Config,
    db::{
        BatchInsertItem, FilterOptions, InsertOptions, MemorySeverity, MemoryStatus, MemoryStore,
        MemoryType, TraverseDirection, UpdateOptions,
    },
    embedding::EmbeddingService,
};

/// Initial instructions for the MCP client (caveman style).
const INSTRUCTIONS: &str = include_str!("instructions.md");

#[derive(Clone)]
pub struct MemServer {
    db_path: Arc<OnceLock<PathBuf>>,
    store: Arc<OnceCell<MemoryStore>>,
    embedding: EmbeddingService,
    config: Arc<Config>,
    shutdown: Arc<Notify>,

    #[allow(dead_code)]
    tool_router: ToolRouter<MemServer>,
}

#[tool_router]
impl MemServer {
    pub fn new(config: Config, embedding: EmbeddingService) -> Self {
        MemServer {
            db_path: Arc::new(OnceLock::new()),
            store: Arc::new(OnceCell::new()),
            embedding,
            config: Arc::new(config),
            shutdown: Arc::new(Notify::new()),
            tool_router: Self::tool_router(),
        }
    }

    /// Serves over stdio and drives the server to completion.
    ///
    /// Returns an error if the transport fails or if a fatal initialization
    /// error occurs (e.g., project path cannot be resolved).
    pub async fn run(self) -> anyhow::Result<()> {
        let shutdown = Arc::clone(&self.shutdown);
        let service = self.serve(rmcp::transport::stdio()).await?;
        tracing::info!("memlite-mcp listening; project path resolves on initialize");

        tokio::select! {
            result = service.waiting() => {
                match &result {
                    Ok(QuitReason::Closed | QuitReason::Cancelled) => {
                        tracing::debug!(reason = ?result, "server stopped");
                    }
                    Ok(QuitReason::JoinError(e)) => {
                        tracing::warn!(error = %e, "server send task failed");
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!(error = %e, "server loop panicked");
                    }
                }
                result.map(|_| ()).map_err(anyhow::Error::from)
            }
            _ = shutdown.notified() => {
                // Yielding once so rmcp's internal send loop can flush the log
                // notification dispatched from on_initialized before we tear down
                tokio::task::yield_now().await;
                Err(anyhow!("exiting: initialization failed (see logs)"))
            }
        }
    }

    /// Returns the cached store, opening (and creating) the DB file on the first call.
    ///
    /// Also validates that the database's recorded embedding model matches the configured
    /// one; returns an error if they differ to prevent mixed-dimension vector corruption.
    async fn open_store(&self) -> Result<&MemoryStore, McpError> {
        let db_path = self.db_path.get().ok_or_else(|| self.path_not_resolved())?;
        let model = self.config.embedding_model;
        self.store
            .get_or_try_init(|| {
                let path = db_path.clone();
                async move {
                    let store = MemoryStore::open(&path)
                        .await
                        .map_err(|e| McpError::from(ServerError::StoreOpen(e)))?;
                    store
                        .check_embedding_model(&model.to_string(), model.dims())
                        .await
                        .map_err(|e| McpError::from(ServerError::StoreOpen(e)))?;
                    Ok(store)
                }
            })
            .await
    }

    /// Returns `Some(store)` when the DB file exists, `None` when it has not been created yet.
    ///
    /// Skips opening for read-only tools so a project with no stored memories never
    /// gets a DB file created on its behalf.
    async fn store_if_present(&self) -> Result<Option<&MemoryStore>, McpError> {
        if let Some(store) = self.store.get() {
            return Ok(Some(store));
        }
        let db_path = self.db_path.get().ok_or_else(|| self.path_not_resolved())?;
        if !db_path.exists() {
            return Ok(None);
        }
        Ok(Some(self.open_store().await?))
    }

    /// Builds an MCP error for when the project path was never resolved at startup.
    fn path_not_resolved(&self) -> McpError {
        McpError::from(ServerError::PathNotResolved(
            "store unavailable: project path not yet resolved".to_owned(),
        ))
    }

    /// Embeds `content` and inserts a new memory row, returning its row identifier.
    #[tool(
        description = "Store a new memory. Embeds the content locally and persists it. Returns the assigned numeric id. Provide summary when content is long or not self-describing. Provide tags when the topic is clear (e.g. architecture, auth, bug)."
    )]
    async fn memory_store(
        &self,
        Parameters(p): Parameters<StoreParams>,
    ) -> Result<CallToolResult, McpError> {
        // Validating all cheap fields before triggering model load/inference
        if p.content.trim().is_empty() {
            return tool_err("content must not be blank");
        }
        let Some(memory_type) = MemoryType::from_str(&p.memory_type) else {
            return tool_err(format!(
                "unknown memory_type {:?}; expected: user, project, reference, task, finding, issue, decision, or plan",
                p.memory_type
            ));
        };
        let status = match p.status.as_deref() {
            None => None,
            Some(s) => match MemoryStatus::from_str(s) {
                Some(st) => Some(st),
                None => {
                    return tool_err(format!(
                        "unknown status {:?}; expected: open, resolved, wontfix, or deferred",
                        s
                    ));
                }
            },
        };
        let severity = match p.severity.as_deref() {
            None => None,
            Some(s) => match MemorySeverity::from_str(s) {
                Some(sev) => Some(sev),
                None => {
                    return tool_err(format!(
                        "unknown severity {:?}; expected: critical, high, medium, or low",
                        s
                    ));
                }
            },
        };
        let tags = p.tags.unwrap_or_default();
        let relations = match parse_relations(p.relations) {
            Ok(r) => r,
            Err(msg) => return tool_err(msg),
        };
        let embedding = self
            .embedding
            .embed(p.content.clone())
            .await
            .map_err(ServerError::Embed)?;
        let store = self.open_store().await?;
        let id = store
            .insert(
                &p.content,
                &embedding,
                InsertOptions {
                    summary: p.summary.as_deref(),
                    tags: &tags,
                    agent: &self.config.agent,
                    context: p.context.as_deref(),
                    confidence: p.confidence,
                    decay_rate: p.decay_rate,
                    memory_type: Some(memory_type),
                    status,
                    severity,
                    relations: &relations,
                },
            )
            .await
            .map_err(ServerError::StoreOp)?;

        let body = serde_json::json!({ "id": id }).to_string();
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// Embeds each item and inserts all rows in a single transaction.
    ///
    /// All items are embedded in one ONNX inference pass. Returns an array of `{"id"}` objects
    /// in the same order as the input. Fails atomically: if any item is invalid or any insert
    /// fails, nothing is written.
    #[tool(
        description = "Store multiple memories in one call. Embeds all items in a single ONNX pass and inserts them atomically. Returns [{\"id\": ...}] in input order. All succeed or all fail. Maximum 100 items."
    )]
    async fn memory_store_batch(
        &self,
        Parameters(p): Parameters<BatchStoreParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.items.is_empty() {
            return tool_err("items must not be empty");
        }
        if p.items.len() > 100 {
            return tool_err(format!("too many items: {} (maximum 100)", p.items.len()));
        }

        // Validating memory_type, status, and severity for all items before embedding anything
        let mut parsed_types: Vec<Option<MemoryType>> = Vec::with_capacity(p.items.len());
        let mut parsed_statuses: Vec<Option<MemoryStatus>> = Vec::with_capacity(p.items.len());
        let mut parsed_severities: Vec<Option<MemorySeverity>> = Vec::with_capacity(p.items.len());
        let mut parsed_relations: Vec<Vec<(i64, String)>> = Vec::with_capacity(p.items.len());
        for (i, item) in p.items.iter().enumerate() {
            let mt = match item.memory_type.as_deref() {
                None => None,
                Some(s) => match MemoryType::from_str(s) {
                    Some(t) => Some(t),
                    None => {
                        return tool_err(format!(
                            "item {i}: unknown memory_type {s:?}; expected: user, project, reference, task, finding, issue, decision, or plan"
                        ));
                    }
                },
            };
            let st = match item.status.as_deref() {
                None => None,
                Some(s) => match MemoryStatus::from_str(s) {
                    Some(t) => Some(t),
                    None => {
                        return tool_err(format!(
                            "item {i}: unknown status {s:?}; expected: open, resolved, wontfix, or deferred"
                        ));
                    }
                },
            };
            let sev = match item.severity.as_deref() {
                None => None,
                Some(s) => match MemorySeverity::from_str(s) {
                    Some(sv) => Some(sv),
                    None => {
                        return tool_err(format!(
                            "item {i}: unknown severity {s:?}; expected: critical, high, medium, or low"
                        ));
                    }
                },
            };
            let rels = match parse_relations(item.relations.clone()) {
                Ok(r) => r,
                Err(msg) => return tool_err(format!("item {i}: {msg}")),
            };
            parsed_types.push(mt);
            parsed_statuses.push(st);
            parsed_severities.push(sev);
            parsed_relations.push(rels);
        }

        let contents: Vec<String> = p.items.iter().map(|item| item.content.clone()).collect();
        let embeddings = self
            .embedding
            .embed_batch(contents)
            .await
            .map_err(ServerError::Embed)?;

        let agent = self.config.agent.clone();
        let batch_items: Vec<BatchInsertItem> = p
            .items
            .into_iter()
            .zip(embeddings)
            .zip(parsed_types)
            .zip(parsed_statuses)
            .zip(parsed_severities)
            .zip(parsed_relations)
            .map(
                |(((((item, embedding), memory_type), status), severity), relations)| {
                    BatchInsertItem {
                        content: item.content,
                        embedding,
                        summary: item.summary,
                        tags: item.tags.unwrap_or_default(),
                        agent: agent.clone(),
                        context: item.context,
                        confidence: item.confidence.unwrap_or(1.0).clamp(0.0, 1.0),
                        decay_rate: item.decay_rate.unwrap_or(0.0).clamp(0.0, 1.0),
                        memory_type,
                        status,
                        severity,
                        relations,
                    }
                },
            )
            .collect();

        let store = self.open_store().await?;
        let ids = store
            .insert_batch(batch_items)
            .await
            .map_err(ServerError::StoreOp)?;

        let body: Vec<_> = ids
            .iter()
            .map(|id| serde_json::json!({ "id": id }))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&body).unwrap(),
        )]))
    }

    /// Embeds `query` and returns the closest stored memories ordered by cosine distance.
    ///
    /// Skips embedding entirely when no DB file exists yet.
    #[tool(
        description = "Search memories by semantic similarity. Returns the closest matches for the given query, ordered by cosine distance (lower = more similar)."
    )]
    async fn memory_search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.query.trim().is_empty() {
            return tool_err("query must not be blank");
        }
        let memory_type = match parse_memory_type(p.memory_type.as_deref()) {
            Ok(v) => v,
            Err(msg) => return tool_err(msg),
        };
        let status = match parse_memory_status(p.status.as_deref()) {
            Ok(v) => v,
            Err(msg) => return tool_err(msg),
        };
        let severity = match parse_memory_severity(p.severity.as_deref()) {
            Ok(v) => v,
            Err(msg) => return tool_err(msg),
        };
        let limit = p.limit.unwrap_or(10).min(100);
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text("[]")]));
        };
        let embedding = self
            .embedding
            .embed(p.query.clone())
            .await
            .map_err(ServerError::Embed)?;
        let results = store
            .search(
                &p.query,
                &embedding,
                FilterOptions {
                    agent_id: self.config.agent_filter(),
                    tags: p.tags.as_deref(),
                    memory_type,
                    status,
                    severity,
                },
                limit,
            )
            .await
            .map_err(ServerError::StoreOp)?;

        let body: Vec<_> = results
            .into_iter()
            .map(|(m, dist)| {
                serde_json::json!({
                    "id":                   m.id,
                    "content":              m.content,
                    "summary":              m.summary,
                    "tags":                 m.tags,
                    "agent":                m.agent,
                    "created_at":           m.created_at,
                    "updated_at":           m.updated_at,
                    "context":              m.context,
                    "distance":             dist,
                    "effective_confidence": m.effective_confidence(),
                    "age_days":             m.age_days(),
                    "memory_type":          m.memory_type,
                    "status":               m.status,
                    "severity":             m.severity,
                    "relations":            m.relations,
                    "freshness_warning":    m.freshness_warning(),
                })
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&body).unwrap(),
        )]))
    }

    /// Returns memories ordered newest-first with optional tag filtering and pagination.
    #[tool(
        description = "List stored memories, newest first. Supports pagination via limit and offset."
    )]
    async fn memory_list(
        &self,
        Parameters(p): Parameters<ListParams>,
    ) -> Result<CallToolResult, McpError> {
        let memory_type = match parse_memory_type(p.memory_type.as_deref()) {
            Ok(v) => v,
            Err(msg) => return tool_err(msg),
        };
        let status = match parse_memory_status(p.status.as_deref()) {
            Ok(v) => v,
            Err(msg) => return tool_err(msg),
        };
        let severity = match parse_memory_severity(p.severity.as_deref()) {
            Ok(v) => v,
            Err(msg) => return tool_err(msg),
        };
        let limit = p.limit.unwrap_or(20).min(100);
        let offset = p.offset.unwrap_or(0);
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text("[]")]));
        };
        let memories = store
            .list(
                FilterOptions {
                    agent_id: self.config.agent_filter(),
                    tags: p.tags.as_deref(),
                    memory_type,
                    status,
                    severity,
                },
                limit,
                offset,
            )
            .await
            .map_err(ServerError::StoreOp)?;

        let body: Vec<_> = memories
            .into_iter()
            .map(|m| {
                serde_json::json!({
                    "id":                   m.id,
                    "content":              m.content,
                    "summary":              m.summary,
                    "tags":                 m.tags,
                    "agent":                m.agent,
                    "created_at":           m.created_at,
                    "updated_at":           m.updated_at,
                    "context":              m.context,
                    "confidence":           m.confidence,
                    "decay_rate":           m.decay_rate,
                    "last_seen_at":         m.last_seen_at,
                    "effective_confidence": m.effective_confidence(),
                    "age_days":             m.age_days(),
                    "memory_type":          m.memory_type,
                    "status":               m.status,
                    "severity":             m.severity,
                    "relations":            m.relations,
                    "freshness_warning":    m.freshness_warning(),
                })
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&body).unwrap(),
        )]))
    }

    /// Returns each distinct tag with its occurrence count, ordered by frequency.
    #[tool(
        description = "List all tags currently in use, with occurrence counts ordered highest first. Call this before memory_store to reuse existing tags and keep the taxonomy consistent."
    )]
    async fn memory_tags(
        &self,
        Parameters(_p): Parameters<TagsParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text("[]")]));
        };
        let tags = store
            .tags(self.config.agent_filter())
            .await
            .map_err(ServerError::StoreOp)?;

        let body: Vec<_> = tags
            .into_iter()
            .map(|(tag, count)| serde_json::json!({ "tag": tag, "count": count }))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&body).unwrap(),
        )]))
    }

    /// Removes the memory with the given row identifier; returns whether a row was deleted.
    #[tool(
        description = "Delete a memory by its numeric id. Returns whether a memory was actually removed."
    )]
    async fn memory_delete(
        &self,
        Parameters(p): Parameters<DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return tool_err(format!("no memory found with id {}", p.id));
        };
        let deleted = store
            .delete(p.id, self.config.agent_filter())
            .await
            .map_err(ServerError::StoreOp)?;
        if deleted {
            let body = serde_json::json!({ "deleted": true, "id": p.id }).to_string();
            Ok(CallToolResult::success(vec![Content::text(body)]))
        } else {
            tool_err(format!("no memory found with id {}", p.id))
        }
    }

    /// Updates one or more fields of an existing memory, re-embedding when `content` changes.
    ///
    /// Passing `summary: ""` clears the summary field (sets it to NULL).
    #[tool(
        description = "Update an existing memory. Provide id plus any fields to change. Updating content triggers re-embedding. Pass summary as empty string to clear it."
    )]
    async fn memory_update(
        &self,
        Parameters(p): Parameters<UpdateParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.content.is_none()
            && p.summary.is_none()
            && p.tags.is_none()
            && p.context.is_none()
            && p.confidence.is_none()
            && p.decay_rate.is_none()
            && p.memory_type.is_none()
            && p.status.is_none()
            && p.severity.is_none()
            && p.relations.is_none()
        {
            return tool_err(
                "at least one of content, summary, tags, context, confidence, decay_rate, memory_type, status, severity, or relations must be provided",
            );
        }

        let update_status = match p.status.as_deref() {
            None => None,
            Some(s) => match MemoryStatus::from_str(s) {
                Some(st) => Some(st),
                None => {
                    return tool_err(format!(
                        "unknown status {:?}; expected: open, resolved, wontfix, or deferred",
                        s
                    ));
                }
            },
        };
        let update_severity = match p.severity.as_deref() {
            None => None,
            Some(s) => match MemorySeverity::from_str(s) {
                Some(sev) => Some(sev),
                None => {
                    return tool_err(format!(
                        "unknown severity {:?}; expected: critical, high, medium, or low",
                        s
                    ));
                }
            },
        };
        let (update_memory_type, clear_memory_type) = match p.memory_type.as_deref() {
            None => (None, false),
            Some("") => (None, true),
            Some(s) => match MemoryType::from_str(s) {
                Some(mt) => (Some(mt), false),
                None => {
                    return tool_err(format!(
                        "unknown memory_type {s:?}; expected: user, project, reference, task, finding, issue, decision, or plan"
                    ));
                }
            },
        };
        let Some(store) = self.store_if_present().await? else {
            return tool_err(format!("no memory found with id {}", p.id));
        };
        let new_embedding = if let Some(ref c) = p.content {
            Some(
                self.embedding
                    .embed(c.clone())
                    .await
                    .map_err(ServerError::Embed)?,
            )
        } else {
            None
        };

        let update_relations = match p.relations.map(|m| parse_relations(Some(m))) {
            None => None,
            Some(Ok(r)) => Some(r),
            Some(Err(msg)) => return tool_err(msg),
        };
        let updated = store
            .update(
                p.id,
                UpdateOptions {
                    content: p.content.as_deref(),
                    summary: p.summary.as_deref(),
                    tags: p.tags.as_deref(),
                    embedding: new_embedding.as_deref(),
                    context: p.context.as_deref(),
                    confidence: p.confidence,
                    decay_rate: p.decay_rate,
                    memory_type: update_memory_type,
                    clear_memory_type,
                    status: update_status,
                    clear_status: matches!(p.status.as_deref(), Some("")),
                    severity: update_severity,
                    clear_severity: matches!(p.severity.as_deref(), Some("")),
                    relations: update_relations.as_deref(),
                },
                self.config.agent_filter(),
            )
            .await
            .map_err(ServerError::StoreOp)?;

        if updated {
            let body = serde_json::json!({ "updated": true, "id": p.id }).to_string();
            Ok(CallToolResult::success(vec![Content::text(body)]))
        } else {
            tool_err(format!("no memory found with id {}", p.id))
        }
    }

    /// Resets the decay clock on multiple memories in a single operation.
    #[tool(
        description = "Reset the decay clock (last_seen_at = now) on a list of memory ids without changing any other field. Returns the count of memories actually updated. Use after confirming multiple memories are still accurate in the same session."
    )]
    async fn memory_revalidate(
        &self,
        Parameters(p): Parameters<RevalidateParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.is_empty() {
            return tool_err("ids must not be empty");
        }
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({ "revalidated": 0 }).to_string(),
            )]));
        };
        let count = store
            .revalidate_batch(&p.ids, self.config.agent_filter())
            .await
            .map_err(ServerError::StoreOp)?;
        let body = serde_json::json!({ "revalidated": count }).to_string();
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// Returns the direct neighbors of a memory: nodes it links to and nodes that link to it.
    #[tool(
        description = "Returns immediate in-neighbors and out-neighbors of a memory node. Useful for exploring what a memory directly references and what references it. Optional kind filter restricts to one edge type."
    )]
    async fn memory_neighbors(
        &self,
        Parameters(p): Parameters<NeighborsParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text(
                "{\"outgoing\":[],\"incoming\":[]}",
            )]));
        };
        let neighbors = store
            .neighbors(p.id, p.kind.as_deref())
            .await
            .map_err(ServerError::StoreOp)?;

        fn format_neighbor(n: crate::db::NeighborInfo) -> serde_json::Value {
            let mem = n.memory.map(|m| {
                serde_json::json!({
                    "id":                   m.id,
                    "summary":              m.summary,
                    "memory_type":          m.memory_type,
                    "status":               m.status,
                    "effective_confidence": m.effective_confidence(),
                })
            });
            serde_json::json!({
                "id":     n.other_id,
                "kind":   n.kind,
                "memory": mem,
            })
        }

        let body = serde_json::json!({
            "outgoing": neighbors.outgoing.into_iter().map(format_neighbor).collect::<Vec<_>>(),
            "incoming": neighbors.incoming.into_iter().map(format_neighbor).collect::<Vec<_>>(),
        });
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }

    /// BFS traversal from a starting memory node.
    #[tool(
        description = "BFS traversal from a memory node up to a given depth. Returns all reachable nodes and traversed edges. direction: 'out' (default) follows outgoing edges, 'in' follows incoming, 'both' follows all. Optional kind filter restricts which edges are traversed."
    )]
    async fn memory_traverse(
        &self,
        Parameters(p): Parameters<TraverseParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({"start_id": p.start_id, "nodes": [], "edges": []}).to_string(),
            )]));
        };
        let direction = parse_direction(p.direction.as_deref());
        let depth = p.depth.unwrap_or(2);
        let result = store
            .traverse(p.start_id, depth, p.kind.as_deref(), direction)
            .await
            .map_err(ServerError::StoreOp)?;

        let nodes: Vec<_> = result
            .nodes
            .into_iter()
            .map(|n| {
                let m = n.memory;
                serde_json::json!({
                    "id":                   m.id,
                    "depth":                n.depth,
                    "via_kind":             n.via_kind,
                    "summary":              m.summary,
                    "memory_type":          m.memory_type,
                    "status":               m.status,
                    "effective_confidence": m.effective_confidence(),
                    "tags":                 m.tags,
                })
            })
            .collect();
        let edges: Vec<_> = result
            .edges
            .into_iter()
            .map(|(f, t, k)| serde_json::json!({"from_id": f, "to_id": t, "kind": k}))
            .collect();
        let body = serde_json::json!({"start_id": result.start_id, "nodes": nodes, "edges": edges});
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }

    /// Finds the shortest directed path between two memory nodes.
    #[tool(
        description = "Finds the shortest directed path between two memory nodes via outgoing relations. Returns the sequence of memories and relation kinds. Use to understand how one concept connects to another through the relation graph."
    )]
    async fn memory_path(
        &self,
        Parameters(p): Parameters<PathParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({"found": false, "path": []}).to_string(),
            )]));
        };
        let max_hops = p.max_hops.unwrap_or(6);
        let result = store
            .path(p.from_id, p.to_id, max_hops)
            .await
            .map_err(ServerError::StoreOp)?;

        let body = match result {
            None => serde_json::json!({"found": false, "path": []}),
            Some(steps) => {
                let path: Vec<_> = steps
                    .into_iter()
                    .map(|s| {
                        let m = s.memory;
                        serde_json::json!({
                            "id":        m.id,
                            "via_kind":  s.via_kind,
                            "summary":   m.summary,
                            "memory_type": m.memory_type,
                            "effective_confidence": m.effective_confidence(),
                        })
                    })
                    .collect();
                serde_json::json!({"found": true, "path": path})
            }
        };
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }

    /// Returns the most-connected memories ranked by total relation degree.
    #[tool(
        description = "Returns the top-N most connected memory nodes by total relation degree (outgoing + incoming). These are the 'hub' memories that the rest of the graph revolves around. Useful for orientation at the start of a session."
    )]
    async fn memory_hubs(
        &self,
        Parameters(p): Parameters<HubsParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text("[]")]));
        };
        let top_n = p.top_n.unwrap_or(10);
        let entries = store.hubs(top_n).await.map_err(ServerError::StoreOp)?;
        let body: Vec<_> = entries
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "id":           e.id,
                    "summary":      e.summary,
                    "memory_type":  e.memory_type,
                    "out_degree":   e.out_degree,
                    "in_degree":    e.in_degree,
                    "total_degree": e.out_degree + e.in_degree,
                })
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&body).unwrap(),
        )]))
    }

    /// Returns graph-level aggregate statistics.
    #[tool(
        description = "Returns graph-level statistics: total memory count (scoped to this agent), breakdown by memory type, total relation edges, and distinct relation kind labels. Good starting point for understanding the shape of the memory graph."
    )]
    async fn memory_stats(
        &self,
        Parameters(_p): Parameters<GraphStatsParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::json!({
                    "total_memories": 0,
                    "by_type": [],
                    "total_relations": 0,
                    "relation_kinds": [],
                })
                .to_string(),
            )]));
        };
        let stats = store
            .graph_stats(self.config.agent_filter())
            .await
            .map_err(ServerError::StoreOp)?;
        let by_type: Vec<_> = stats
            .by_type
            .into_iter()
            .map(|(k, v)| serde_json::json!({"type": k, "count": v}))
            .collect();
        let body = serde_json::json!({
            "total_memories":  stats.total_memories,
            "by_type":         by_type,
            "total_relations": stats.total_relations,
            "relation_kinds":  stats.relation_kinds,
        });
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for MemServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("memlite", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS.to_string())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        // Guarding against double-initialization
        if self.db_path.get().is_some() {
            return;
        }

        // Skipping if the client didn't advertise roots support; calling list_roots
        // on such a client would hang waiting for a response that never comes
        let peer = context.peer.peer_info();
        tracing::debug!(
            has_peer_info = peer.is_some(),
            roots_capability = ?peer.and_then(|i| i.capabilities.roots.as_ref()),
            "on_initialized: peer capabilities"
        );
        let supports_roots = peer.is_some_and(|info| info.capabilities.roots.is_some());

        const LIST_ROOTS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

        let root_uris = if supports_roots {
            match tokio::time::timeout(LIST_ROOTS_TIMEOUT, context.peer.list_roots()).await {
                Ok(Ok(result)) => result.roots.into_iter().map(|r| r.uri).collect::<Vec<_>>(),
                Ok(Err(e)) => {
                    tracing::warn!("list_roots failed: {e}");
                    vec![]
                }
                Err(_) => {
                    tracing::warn!(
                        "list_roots timed out after {:?}; falling back to CWD",
                        LIST_ROOTS_TIMEOUT
                    );
                    vec![]
                }
            }
        } else {
            tracing::debug!("skipping list_roots: client does not advertise roots capability");
            vec![]
        };

        tracing::debug!(root_uris = ?root_uris, "resolved root URIs");
        let project_path = match self.config.resolve_project_path(&root_uris) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("fatal: {e}");
                // Sending a best-effort log notification so the client sees why the
                // server is about to disconnect; ignored if the client hasn't subscribed
                let _ = context
                    .peer
                    .notify_logging_message(LoggingMessageNotificationParam::new(
                        LoggingLevel::Error,
                        serde_json::json!(e.to_string()),
                    ))
                    .await;
                self.shutdown.notify_one();
                return;
            }
        };

        let db_path = self.config.db_path_for(&project_path);
        tracing::info!(path = ?db_path, project = %project_path, "resolved database path");
        if self.db_path.set(db_path).is_err() {
            tracing::warn!("db path already set");
        }
    }
}

/// Wraps a message as a tool-level error result (distinct from an MCP protocol error).
fn tool_err(msg: impl std::fmt::Display) -> Result<CallToolResult, McpError> {
    let mut result = CallToolResult::success(vec![Content::text(msg.to_string())]);
    result.is_error = Some(true);
    Ok(result)
}

/// Converts an optional relations map into a validated list of `(target_id, kind)` pairs.
///
/// Returns an error string when any key is not a valid i64 or any kind value is empty.
fn parse_relations(
    map: Option<std::collections::BTreeMap<String, String>>,
) -> Result<Vec<(i64, String)>, String> {
    let Some(map) = map else {
        return Ok(Vec::new());
    };
    let mut result = Vec::with_capacity(map.len());
    for (key, kind) in map {
        let id = key.parse::<i64>().map_err(|_| {
            format!("relations key {key:?} is not a valid memory id; expected a numeric id")
        })?;
        if kind.trim().is_empty() {
            return Err(format!("relations[{key}]: kind must be a non-empty string"));
        }
        result.push((id, kind));
    }
    Ok(result)
}

/// Converts an optional type string into a [`MemoryType`], returning `None` for unrecognised values.
/// Parses an optional memory type filter, returning an error string when a value is provided but unrecognised.
///
/// `None` input means no filter; `Some` with an unrecognised value is an error rather than a no-op.
fn parse_memory_type(s: Option<&str>) -> Result<Option<MemoryType>, String> {
    match s {
        None => Ok(None),
        Some(v) => MemoryType::from_str(v).map(Some).ok_or_else(|| {
            format!("unknown memory_type {v:?}; expected: user, project, reference, task, finding, issue, decision, or plan")
        }),
    }
}

/// Parses an optional status filter, returning an error string when a value is provided but unrecognised.
fn parse_memory_status(s: Option<&str>) -> Result<Option<MemoryStatus>, String> {
    match s {
        None => Ok(None),
        Some(v) => MemoryStatus::from_str(v).map(Some).ok_or_else(|| {
            format!("unknown status {v:?}; expected: open, resolved, wontfix, or deferred")
        }),
    }
}

/// Converts a direction string into a [`TraverseDirection`]; defaults to `Out` for unrecognised values.
fn parse_direction(s: Option<&str>) -> TraverseDirection {
    match s {
        Some("in") => TraverseDirection::In,
        Some("both") => TraverseDirection::Both,
        _ => TraverseDirection::Out,
    }
}

/// Parses an optional severity filter, returning an error string when a value is provided but unrecognised.
fn parse_memory_severity(s: Option<&str>) -> Result<Option<MemorySeverity>, String> {
    match s {
        None => Ok(None),
        Some(v) => MemorySeverity::from_str(v).map(Some).ok_or_else(|| {
            format!("unknown severity {v:?}; expected: critical, high, medium, or low")
        }),
    }
}

/// Typed errors produced at the MCP boundary, converted to [`McpError`] via [`From`].
enum ServerError {
    /// Project path was not resolved during initialisation; store cannot be located.
    PathNotResolved(String),
    /// [`MemoryStore::open`] failed (permissions, corruption, disk full, etc.).
    StoreOpen(anyhow::Error),
    /// A database query or write operation failed after the store was open.
    StoreOp(anyhow::Error),
    /// The embedding model failed to produce a vector for the given text.
    Embed(anyhow::Error),
}

impl From<ServerError> for McpError {
    fn from(e: ServerError) -> McpError {
        McpError::internal_error(
            match e {
                ServerError::PathNotResolved(msg) => msg,
                ServerError::StoreOpen(e) => format!("failed to open database: {e}"),
                ServerError::StoreOp(e) => format!("database operation failed: {e}"),
                ServerError::Embed(e) => format!("embedding failed: {e}"),
            },
            None,
        )
    }
}
