use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
};

use super::params::{
    DeleteParams, EmptyParams, HubsParams, ListParams, NeighborsParams, PathParams,
    RevalidateParams, SearchParams, StoreParams, TraverseParams, UpdateParams,
};
use crate::{
    db::{
        BatchInsertItem, FilterOptions, MemoryAttrOutcome, MemoryAttrPhase, MemoryAttrPriority,
        MemoryAttrSeverity, MemoryStatus, MemoryStore, MemoryType, TraverseDirection,
        UpdateOptions,
    },
    embedding::EmbeddingService,
    output::{
        hubs_json, neighbors_json, path_json, render_compact_list_results, render_list_results,
        render_search_results, render_toon, stats_json, traverse_json,
    },
    worker::ProjectHandle,
};

/// Initial instructions for the MCP client (caveman style).
const INSTRUCTIONS: &str = include_str!("instructions.md");

// Parses an Optional<String> filter field into the typed enum, returning early on error.
macro_rules! parse_filter {
    ($opt:expr, $ty:ty) => {
        match $opt.as_deref().map(|s| s.parse::<$ty>()).transpose() {
            Ok(v) => v,
            Err(e) => return tool_err(e),
        }
    };
}

pub struct MemServer {
    project: Arc<ProjectHandle>,

    #[allow(dead_code)]
    tool_router: ToolRouter<MemServer>,
}

#[tool_router]
impl MemServer {
    pub(crate) fn new(project: Arc<ProjectHandle>) -> Self {
        Self {
            project,
            tool_router: Self::tool_router(),
        }
    }

    /// Opens or returns project storage.
    async fn open_store(&self) -> Result<&MemoryStore, McpError> {
        self.project
            .open_store()
            .await
            .map_err(ServerError::StoreOpen)
            .map_err(McpError::from)
    }

    /// Returns project storage only when its database exists.
    async fn store_if_present(&self) -> Result<Option<&MemoryStore>, McpError> {
        self.project
            .store_if_present()
            .await
            .map_err(ServerError::StoreOpen)
            .map_err(McpError::from)
    }

    /// Returns project embedding inference.
    fn embedding(&self) -> &EmbeddingService {
        self.project.embedding()
    }

    /// Embeds each item and inserts all rows in a single transaction.
    ///
    /// Embeds all items through one batched FastEmbed call. Returns identifiers in input order.
    /// Fails atomically: if any item is invalid or any insert fails, nothing is written.
    #[tool(
        description = "Store one or more memories in one call. Before storing, call memory_tags to reuse the taxonomy, then memory_search for the same concept; if distance < 0.15, update the existing memory instead. Store only information that will save meaningful future effort. Embeds all items through one batched FastEmbed call and inserts them atomically. Returns {\"ids\":[...]} in input order. All succeed or all fail. Maximum 100 items."
    )]
    async fn memory_store(
        &self,
        Parameters(mut p): Parameters<StoreParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.items.is_empty() {
            return tool_err("items must not be empty");
        }
        if p.items.len() > 100 {
            return tool_err(format!("too many items: {} (maximum 100)", p.items.len()));
        }

        // Validating cheap fields before embedding anything
        let mut parsed_types: Vec<MemoryType> = Vec::with_capacity(p.items.len());
        let mut parsed_relations: Vec<Vec<(i64, String)>> = Vec::with_capacity(p.items.len());
        for (i, item) in p.items.iter_mut().enumerate() {
            if item.content.trim().is_empty() {
                return tool_err(format!("item {i}: content must not be blank"));
            }
            if item.summary.trim().is_empty() {
                return tool_err(format!("item {i}: summary must not be blank"));
            }
            let mt = match item.memory_type.parse::<MemoryType>() {
                Ok(v) => v,
                Err(e) => return tool_err(format!("item {i}: {e}")),
            };
            let rels = match parse_relations(item.relations.take()) {
                Ok(r) => r,
                Err(msg) => return tool_err(format!("item {i}: {msg}")),
            };
            parsed_types.push(mt);
            parsed_relations.push(rels);
        }

        // Moving content through inference to avoid cloning every stored string
        let contents = p
            .items
            .iter_mut()
            .map(|item| std::mem::take(&mut item.content))
            .collect();
        let (contents, embeddings) = self
            .embedding()
            .embed_batch_owned(contents)
            .await
            .map_err(ServerError::Embed)?;

        // Assembling values from vectors whose lengths were checked during embedding
        let mut contents = contents.into_iter();
        let mut embeddings = embeddings.into_iter();
        let mut parsed_types = parsed_types.into_iter();
        let mut parsed_relations = parsed_relations.into_iter();
        let batch_items: Vec<BatchInsertItem> = p
            .items
            .into_iter()
            .map(|item| BatchInsertItem {
                content: contents.next().unwrap(),
                embedding: embeddings.next().unwrap(),
                summary: item.summary,
                tags: item.tags.unwrap_or_default(),
                context: item.context,
                confidence: item.confidence.unwrap_or(1.0),
                decay_rate: item.decay_rate.unwrap_or(0.0),
                memory_type: parsed_types.next().unwrap(),
                attrs: item.attrs.unwrap_or_default(),
                relations: parsed_relations.next().unwrap(),
                refs: item.refs.unwrap_or_default(),
            })
            .collect();

        let store = self.open_store().await?;
        let ids = store
            .insert_batch(batch_items)
            .await
            .map_err(ServerError::StoreOp)?;

        tool_ok(serde_json::json!({ "ids": ids }))
    }

    /// Embeds `query` and returns the closest stored memories ordered by cosine distance.
    ///
    /// Skips embedding entirely when no DB file exists yet.
    #[tool(
        description = "Search memories by semantic similarity before investigating, debugging, or implementing. Returns closest matches ordered by cosine distance: <0.15 likely same concept, <0.40 related, 0.40-0.70 weak, >0.70 unrelated. effective_confidence >=0.8 is reliable, 0.5-0.8 warrants caution, <0.5 is potentially stale. Content is omitted by default; compact results include the required summary. Pass include_content=true for full context and content; full results omit redundant summary."
    )]
    async fn memory_search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.query.trim().is_empty() {
            return tool_err("query must not be blank");
        }
        let memory_type = parse_filter!(p.memory_type, MemoryType);
        let status = parse_filter!(p.status, MemoryStatus);
        let severity = parse_filter!(p.severity, MemoryAttrSeverity);
        let priority = parse_filter!(p.priority, MemoryAttrPriority);
        let outcome = parse_filter!(p.outcome, MemoryAttrOutcome);
        let phase = parse_filter!(p.phase, MemoryAttrPhase);
        let limit = p.limit.unwrap_or(10).min(100);
        let Some(store) = self.store_if_present().await? else {
            return tool_text("memories: 0".to_owned());
        };
        let (query, embedding) = self
            .embedding()
            .embed_owned(p.query)
            .await
            .map_err(ServerError::Embed)?;
        let results = store
            .search(
                &query,
                &embedding,
                FilterOptions {
                    ids: None,
                    tags: p.tags.as_deref(),
                    memory_type,
                    status,
                    severity,
                    priority,
                    outcome,
                    phase,
                    refs_contain: p.refs_contain.as_deref(),
                },
                limit,
            )
            .await
            .map_err(ServerError::StoreOp)?;

        tool_text(render_search_results(
            &results,
            p.include_content.unwrap_or(false),
        ))
    }

    /// Returns memories ordered newest-first with optional tag filtering and pagination.
    #[tool(
        description = "List stored memories, newest first. effective_confidence >=0.8 is reliable, 0.5-0.8 warrants caution, <0.5 is potentially stale. Supports pagination via limit and offset. include_content defaults to true and full results omit redundant summary. Pass false for compact selection output; fetch selected full memories with memory_list(ids=[...])."
    )]
    async fn memory_list(
        &self,
        Parameters(p): Parameters<ListParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.ids.as_ref().is_some_and(|ids| ids.len() > 100) {
            return tool_err("too many ids (maximum 100)");
        }
        let memory_type = parse_filter!(p.memory_type, MemoryType);
        let status = parse_filter!(p.status, MemoryStatus);
        let severity = parse_filter!(p.severity, MemoryAttrSeverity);
        let priority = parse_filter!(p.priority, MemoryAttrPriority);
        let outcome = parse_filter!(p.outcome, MemoryAttrOutcome);
        let phase = parse_filter!(p.phase, MemoryAttrPhase);
        let limit = p
            .limit
            .unwrap_or_else(|| p.ids.as_ref().map_or(20, |ids| ids.len() as u32))
            .min(100);
        let offset = p.offset.unwrap_or(0);
        let Some(store) = self.store_if_present().await? else {
            return tool_text("memories: 0".to_owned());
        };
        let filter = FilterOptions {
            ids: p.ids.as_deref(),
            tags: p.tags.as_deref(),
            memory_type,
            status,
            severity,
            priority,
            outcome,
            phase,
            refs_contain: p.refs_contain.as_deref(),
        };
        if p.include_content.unwrap_or(true) {
            let memories = store
                .list(filter, limit, offset)
                .await
                .map_err(ServerError::StoreOp)?;
            tool_text(render_list_results(&memories, true))
        } else {
            let memories = store
                .list_compact(filter, limit, offset)
                .await
                .map_err(ServerError::StoreOp)?;
            tool_text(render_compact_list_results(&memories))
        }
    }

    /// Returns each distinct tag with its occurrence count, ordered by frequency.
    #[tool(
        description = "Return all tags as a compact tag-to-count object. Call this before memory_store to reuse existing tags and keep the taxonomy consistent."
    )]
    async fn memory_tags(
        &self,
        Parameters(_p): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return tool_ok(serde_json::json!({}));
        };
        let tags = store.tags().await.map_err(ServerError::StoreOp)?;

        let body = serde_json::Value::Object(
            tags.into_iter()
                .map(|(tag, count)| (tag, serde_json::Value::from(count)))
                .collect(),
        );
        tool_ok(body)
    }

    /// Removes the memory with the given row identifier; returns whether a row was deleted.
    #[tool(
        description = "Delete a memory by its numeric id only when it is fully obsolete or factually wrong. Preserve completed history by using memory_update with attrs.status=resolved for resolved tasks, findings, and issues. Returns whether a memory was actually removed."
    )]
    async fn memory_delete(
        &self,
        Parameters(p): Parameters<DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        let deleted = match self.store_if_present().await? {
            Some(store) => store.delete(p.id).await.map_err(ServerError::StoreOp)?,
            None => false,
        };
        tool_ok(serde_json::json!({ "deleted": deleted, "id": p.id }))
    }

    /// Updates one or more fields of an existing memory, re-embedding when `content` changes.
    #[tool(
        description = "Update an existing memory. Provide id plus any fields to change. Updating content triggers re-embedding. Blank summaries are rejected."
    )]
    async fn memory_update(
        &self,
        Parameters(mut p): Parameters<UpdateParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.content.is_none()
            && p.summary.is_none()
            && p.tags.is_none()
            && p.context.is_none()
            && p.confidence.is_none()
            && p.decay_rate.is_none()
            && p.memory_type.is_none()
            && p.attrs.is_none()
            && p.relations.is_none()
            && p.refs.is_none()
        {
            return tool_err(
                "at least one of content, summary, tags, context, confidence, decay_rate, memory_type, attrs, relations, or refs must be provided",
            );
        }

        if p.content
            .as_ref()
            .is_some_and(|content| content.trim().is_empty())
        {
            return tool_err("content must not be blank");
        }
        if p.summary
            .as_ref()
            .is_some_and(|summary| summary.trim().is_empty())
        {
            return tool_err("summary must not be blank");
        }

        let update_attrs = p.attrs;
        let update_memory_type = match p.memory_type.as_deref() {
            None => None,
            Some("") => return tool_err("memory_type must not be blank"),
            Some(s) => match s.parse::<MemoryType>() {
                Ok(mt) => Some(mt),
                Err(e) => return tool_err(e),
            },
        };
        let update_relations = match p.relations.take().map(|m| parse_relations(Some(m))) {
            None => None,
            Some(Ok(relations)) => Some(relations),
            Some(Err(message)) => return tool_err(message),
        };
        let Some(store) = self.store_if_present().await? else {
            return tool_err(format!("no memory found with id {}", p.id));
        };
        if !store.contains(p.id).await.map_err(ServerError::StoreOp)? {
            return tool_err(format!("no memory found with id {}", p.id));
        }

        // Moving changed content through inference without an extra allocation
        let new_embedding = if let Some(content) = p.content.take() {
            let (content, embedding) = self
                .embedding()
                .embed_owned(content)
                .await
                .map_err(ServerError::Embed)?;
            p.content = Some(content);
            Some(embedding)
        } else {
            None
        };
        let update_refs = p.refs.as_deref();
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
                    attrs: update_attrs,
                    relations: update_relations.as_deref(),
                    refs: update_refs,
                },
            )
            .await
            .map_err(ServerError::StoreOp)?;

        if updated {
            tool_ok(serde_json::json!({ "updated": true, "id": p.id }))
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
        if p.ids.len() > 100 {
            return tool_err("too many ids (maximum 100)");
        }
        let Some(store) = self.store_if_present().await? else {
            return tool_ok(serde_json::json!({ "revalidated": 0 }));
        };
        let count = store
            .revalidate_batch(&p.ids)
            .await
            .map_err(ServerError::StoreOp)?;
        tool_ok(serde_json::json!({ "revalidated": count }))
    }

    /// Returns the direct neighbors of a memory: nodes it links to and nodes that link to it.
    #[tool(
        description = "Returns immediate in-neighbors and out-neighbors as compact selection rows with id, kind, memory_type, and summary. Optional kind filter restricts to one edge type. Fetch selected full memories with memory_list(ids=[...])."
    )]
    async fn memory_neighbors(
        &self,
        Parameters(p): Parameters<NeighborsParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return tool_ok(serde_json::json!({"outgoing": [], "incoming": []}));
        };
        let neighbors = store
            .neighbors(p.id, p.kind.as_deref())
            .await
            .map_err(ServerError::StoreOp)?;

        tool_ok(neighbors_json(neighbors))
    }

    /// BFS traversal from a starting memory node.
    #[tool(
        description = "BFS traversal from a memory node up to a given depth. Returns compact node selection rows and traversed edges. direction: 'out' (default) follows outgoing relations, 'in' follows incoming, 'both' follows all. Optional kind filter restricts which edges are traversed. Fetch selected full memories with memory_list(ids=[...])."
    )]
    async fn memory_traverse(
        &self,
        Parameters(p): Parameters<TraverseParams>,
    ) -> Result<CallToolResult, McpError> {
        let direction = match p.direction.as_deref() {
            None | Some("out") => TraverseDirection::Out,
            Some("in") => TraverseDirection::In,
            Some("both") => TraverseDirection::Both,
            Some(_) => return tool_err("direction must be out, in, or both"),
        };
        let Some(store) = self.store_if_present().await? else {
            return tool_ok(serde_json::json!({"start_id": p.start_id, "nodes": [], "edges": []}));
        };
        let depth = p.depth.unwrap_or(2);
        let result = store
            .traverse(p.start_id, depth, p.kind.as_deref(), direction)
            .await
            .map_err(ServerError::StoreOp)?;

        tool_ok(traverse_json(result))
    }

    /// Finds the shortest directed path between two memory nodes.
    #[tool(
        description = "Finds the shortest directed path between two memory nodes via outgoing relations. Returns compact memory selection rows and relation kinds. Use to understand how one concept connects to another, then fetch selected full memories with memory_list(ids=[...])."
    )]
    async fn memory_path(
        &self,
        Parameters(p): Parameters<PathParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return tool_ok(serde_json::json!({"found": false, "path": []}));
        };
        let max_hops = p.max_hops.unwrap_or(6);
        let result = store
            .path(p.from_id, p.to_id, max_hops)
            .await
            .map_err(ServerError::StoreOp)?;

        tool_ok(path_json(result))
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
            return tool_ok(serde_json::json!([]));
        };
        let top_n = p.top_n.unwrap_or(10);
        let entries = store.hubs(top_n).await.map_err(ServerError::StoreOp)?;
        tool_ok(hubs_json(entries))
    }

    /// Returns graph-level aggregate statistics.
    #[tool(
        description = "Returns graph-level statistics: total memory count, breakdown by memory type, total relation edges, and distinct relation kind labels. Counts are global (not scoped to an agent), matching the rest of the graph tools. Good starting point for understanding the shape of the memory graph."
    )]
    async fn memory_stats(
        &self,
        Parameters(_p): Parameters<EmptyParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(store) = self.store_if_present().await? else {
            return tool_ok(serde_json::json!({
                "total_memories": 0,
                "by_type": {},
                "total_relations": 0,
                "relation_kinds": [],
            }));
        };
        let stats = store.graph_stats().await.map_err(ServerError::StoreOp)?;
        tool_ok(stats_json(stats))
    }
}

#[tool_handler]
impl ServerHandler for MemServer {
    fn get_info(&self) -> ServerInfo {
        server_info()
    }
}

pub(crate) fn server_info() -> ServerInfo {
    ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new("memlite", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS.to_string())
}

/// Encodes compact structured output as TOON text.
fn tool_ok(value: serde_json::Value) -> Result<CallToolResult, McpError> {
    let body = render_toon(value).map_err(ServerError::ToonEncode)?;
    tool_text(body)
}

/// Wraps already-formatted successful tool text.
fn tool_text(body: String) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::success(vec![Content::text(body)]))
}

/// Wraps a message as a tool-level error result (distinct from an MCP protocol error).
fn tool_err(msg: impl std::fmt::Display) -> Result<CallToolResult, McpError> {
    Ok(CallToolResult::error(vec![Content::text(msg.to_string())]))
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
            format!("relations key {key} is not a valid memory id; expected a numeric id")
        })?;
        if id <= 0 {
            return Err(format!(
                "relations key {key} is not a valid memory id; expected a positive numeric id"
            ));
        }
        if kind.trim().is_empty() {
            return Err(format!("relations[{key}]: kind must be a non-empty string"));
        }
        result.push((id, kind));
    }
    Ok(result)
}

/// Typed errors produced at the MCP boundary, converted to [`McpError`] via [`From`].
enum ServerError {
    /// [`MemoryStore::open`] failed (permissions, corruption, disk full, etc.).
    StoreOpen(anyhow::Error),
    /// A database query or write operation failed after the store was open.
    StoreOp(anyhow::Error),
    /// The embedding model failed to produce a vector for the given text.
    Embed(anyhow::Error),
    /// TOON output encoding failure.
    ToonEncode(serde_toon::Error),
}

impl From<ServerError> for McpError {
    fn from(e: ServerError) -> McpError {
        McpError::internal_error(
            match e {
                ServerError::StoreOpen(e) => format!("failed to open database: {e:#}"),
                ServerError::StoreOp(e) => format!("database operation failed: {e:#}"),
                ServerError::Embed(e) => format!("embedding failed: {e:#}"),
                ServerError::ToonEncode(e) => format!("TOON encoding failed: {e}"),
            },
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::{
        cli::Cli,
        mcp::params::StoreItem,
        worker::{ProjectSpec, Registry},
    };

    async fn test_server() -> MemServer {
        let Cli { config, .. } = Cli::try_parse_from([
            "memlite-mcp",
            "serve",
            "--project-path",
            "/tmp/memlite-mcp-server-test",
            "--db-dir",
            "/tmp/memlite-mcp-server-test-db",
        ])
        .unwrap();
        let project_path = config.resolve_project_path(&[]).unwrap();
        let spec = ProjectSpec::new(&config, project_path);
        let project = Registry::default().project(&spec).unwrap();
        MemServer::new(project)
    }

    fn result_text(result: &CallToolResult) -> &str {
        let RawContent::Text(content) = &result.content[0].raw else {
            panic!("expected text tool content")
        };
        &content.text
    }

    #[test]
    fn server_errors_preserve_causes() {
        for error in [
            ServerError::StoreOpen(anyhow::anyhow!("disk full").context("opening store")),
            ServerError::StoreOp(anyhow::anyhow!("disk full").context("writing memory")),
            ServerError::Embed(anyhow::anyhow!("model failed").context("embedding task")),
        ] {
            let error = McpError::from(error);
            assert!(error.message.contains("disk full") || error.message.contains("model failed"));
        }
    }

    #[test]
    fn structured_tool_output_compacts_and_round_trips_as_toon() {
        for (input, expected) in [
            (serde_json::json!({}), serde_json::json!({})),
            (serde_json::json!([]), serde_json::json!([])),
            (
                serde_json::json!({"ids": [1, 2]}),
                serde_json::json!({"ids": [1, 2]}),
            ),
            (
                serde_json::json!({
                    "rows": [{"id": 1, "attrs": {}, "summary": null}],
                    "empty": [],
                    "nested": {"empty": {}, "missing": null},
                    "zero": 0,
                    "blank": "",
                }),
                serde_json::json!({"rows": [{"id": 1}], "zero": 0, "blank": ""}),
            ),
        ] {
            let result = tool_ok(input).unwrap();
            let actual: serde_json::Value = serde_toon::from_str(result_text(&result)).unwrap();
            assert_eq!(actual, expected);
        }
    }

    #[tokio::test]
    async fn memory_store_rejects_blank_summary_before_embedding() {
        let result = test_server()
            .await
            .memory_store(Parameters(StoreParams {
                items: vec![StoreItem {
                    content: "content".to_owned(),
                    summary: "  ".to_owned(),
                    tags: None,
                    context: None,
                    memory_type: "project".to_owned(),
                    confidence: None,
                    decay_rate: None,
                    attrs: None,
                    relations: None,
                    refs: None,
                }],
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result_text(&result), "item 0: summary must not be blank");
    }

    #[tokio::test]
    async fn memory_delete_reports_missing_memory() {
        let result = test_server()
            .await
            .memory_delete(Parameters(DeleteParams { id: -1 }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false));
        let value: serde_json::Value = serde_toon::from_str(result_text(&result)).unwrap();
        assert_eq!(value, serde_json::json!({"deleted": false, "id": -1}));
    }

    #[tokio::test]
    async fn memory_update_rejects_blank_content_before_opening_store() {
        let result = test_server()
            .await
            .memory_update(Parameters(UpdateParams {
                id: 1,
                content: Some("  ".to_owned()),
                summary: None,
                tags: None,
                context: None,
                confidence: None,
                decay_rate: None,
                memory_type: None,
                attrs: None,
                relations: None,
                refs: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result_text(&result), "content must not be blank");
    }

    #[tokio::test]
    async fn memory_update_rejects_blank_summary_before_opening_store() {
        let result = test_server()
            .await
            .memory_update(Parameters(UpdateParams {
                id: 1,
                content: None,
                summary: Some("  ".to_owned()),
                tags: None,
                context: None,
                confidence: None,
                decay_rate: None,
                memory_type: None,
                attrs: None,
                relations: None,
                refs: None,
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result_text(&result), "summary must not be blank");
    }

    #[tokio::test]
    async fn memory_revalidate_rejects_more_than_one_hundred_ids() {
        let result = test_server()
            .await
            .memory_revalidate(Parameters(RevalidateParams { ids: vec![1; 101] }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result_text(&result), "too many ids (maximum 100)");
    }

    #[tokio::test]
    async fn memory_traverse_rejects_unknown_direction_without_store() {
        let server = test_server().await;
        let result = server
            .memory_traverse(Parameters(TraverseParams {
                start_id: 1,
                depth: None,
                kind: None,
                direction: Some("inn".to_owned()),
            }))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert_eq!(result_text(&result), "direction must be out, in, or both");
    }

    #[test]
    fn parse_relations_rejects_nonpositive_target() {
        let error = parse_relations(Some(std::collections::BTreeMap::from([(
            "0".to_owned(),
            "related_to".to_owned(),
        )])))
        .unwrap_err();
        assert!(error.contains("positive numeric id"));
    }
}
