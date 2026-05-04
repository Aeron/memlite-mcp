use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, serde_json,
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde::Deserialize;

use crate::{
    config::Config,
    db::MemoryStore,
    embedding::EmbeddingService,
};

// Tool parameter types -------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreParams {
    /// The text to store and embed.
    pub content: String,
    /// Short human-readable description of the memory (optional).
    pub summary: Option<String>,
    /// Metadata tags for categorisation (optional JSON array of strings).
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Natural-language query; results are ranked by cosine similarity.
    pub query: String,
    /// Maximum number of results to return (default: 10, max: 100).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Maximum number of results per page (default: 20, max: 100).
    pub limit: Option<u32>,
    /// Zero-based offset for pagination (default: 0).
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteParams {
    /// Numeric identifier of the memory to delete (from memory_store or memory_list).
    pub id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateParams {
    /// Numeric identifier of the memory to update.
    pub id: i64,
    /// Replacement content; triggers re-embedding when provided.
    pub content: Option<String>,
    /// Replacement summary; pass empty string to clear the field.
    pub summary: Option<String>,
    /// Replacement tags.
    pub tags: Option<Vec<String>>,
}

// Server struct --------------------------------------------------------------

#[derive(Clone)]
pub struct MemServer {
    store: MemoryStore,
    embedding: EmbeddingService,
    config: Arc<Config>,
    // Used by the #[tool_handler] macro-generated call_tool / list_tools impls.
    #[allow(dead_code)]
    tool_router: ToolRouter<MemServer>,
}

// Helper for tool-level (non-protocol) error responses
fn tool_err(msg: impl std::fmt::Display) -> Result<CallToolResult, McpError> {
    let mut result = CallToolResult::success(vec![Content::text(msg.to_string())]);
    result.is_error = Some(true);
    Ok(result)
}

fn internal_err(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

#[tool_router]
impl MemServer {
    pub fn new(config: Config, store: MemoryStore, embedding: EmbeddingService) -> Self {
        MemServer {
            store,
            embedding,
            config: Arc::new(config),
            tool_router: Self::tool_router(),
        }
    }

    /// Embeds `content` and persists it as a new memory.
    ///
    /// Returns the numeric identifier assigned to the stored memory.
    #[tool(description = "Store a new memory. Embeds the content locally and persists it. Returns the assigned numeric id.")]
    async fn memory_store(
        &self,
        Parameters(p): Parameters<StoreParams>,
    ) -> Result<CallToolResult, McpError> {
        let embedding = self.embedding.embed(p.content.clone()).await.map_err(internal_err)?;
        let tags = p.tags.unwrap_or_default();
        let id = self
            .store
            .insert(
                &p.content,
                p.summary.as_deref(),
                &tags,
                &self.config.agent_id,
                &embedding,
            )
            .await
            .map_err(internal_err)?;

        let body = serde_json::json!({ "id": id }).to_string();
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// Performs a semantic (cosine similarity) search over stored memories.
    ///
    /// Returns an array of memories ordered nearest-first, each with a
    /// `distance` field (lower is more similar).
    #[tool(description = "Search memories by semantic similarity. Returns the closest matches for the given query, ordered by cosine distance (lower = more similar).")]
    async fn memory_search(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let limit = p.limit.unwrap_or(10).min(100);
        let embedding = self.embedding.embed(p.query).await.map_err(internal_err)?;
        let results = self
            .store
            .search(&embedding, self.config.agent_filter(), limit)
            .await
            .map_err(internal_err)?;

        let body: Vec<_> = results
            .into_iter()
            .map(|(m, dist)| {
                serde_json::json!({
                    "id":         m.id,
                    "content":    m.content,
                    "summary":    m.summary,
                    "tags":       m.tags,
                    "agent_id":   m.agent_id,
                    "created_at": m.created_at,
                    "updated_at": m.updated_at,
                    "distance":   dist,
                })
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(serde_json::to_string(&body).unwrap())]))
    }

    /// Lists memories ordered by creation time (newest first) with pagination.
    #[tool(description = "List stored memories, newest first. Supports pagination via limit and offset.")]
    async fn memory_list(
        &self,
        Parameters(p): Parameters<ListParams>,
    ) -> Result<CallToolResult, McpError> {
        let limit = p.limit.unwrap_or(20).min(100);
        let offset = p.offset.unwrap_or(0);
        let memories = self
            .store
            .list(self.config.agent_filter(), limit, offset)
            .await
            .map_err(internal_err)?;

        let body = serde_json::to_string(&memories).unwrap();
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

    /// Deletes a memory by its numeric identifier.
    #[tool(description = "Delete a memory by its numeric id. Returns whether a memory was actually removed.")]
    async fn memory_delete(
        &self,
        Parameters(p): Parameters<DeleteParams>,
    ) -> Result<CallToolResult, McpError> {
        let deleted = self.store.delete(p.id).await.map_err(internal_err)?;
        if deleted {
            let body = serde_json::json!({ "deleted": true, "id": p.id }).to_string();
            Ok(CallToolResult::success(vec![Content::text(body)]))
        } else {
            tool_err(format!("no memory found with id {}", p.id))
        }
    }

    /// Updates one or more fields of an existing memory.
    ///
    /// Updating `content` automatically re-embeds the new text.
    /// Pass `summary: ""` to clear the summary field.
    #[tool(description = "Update an existing memory. Provide id plus any fields to change. Updating content triggers re-embedding. Pass summary as empty string to clear it.")]
    async fn memory_update(
        &self,
        Parameters(p): Parameters<UpdateParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.content.is_none() && p.summary.is_none() && p.tags.is_none() {
            return tool_err("at least one of content, summary, or tags must be provided");
        }

        let new_embedding = if let Some(ref c) = p.content {
            Some(self.embedding.embed(c.clone()).await.map_err(internal_err)?)
        } else {
            None
        };

        let updated = self
            .store
            .update(
                p.id,
                p.content.as_deref(),
                p.summary.as_deref(),
                p.tags.as_deref(),
                new_embedding.as_deref(),
            )
            .await
            .map_err(internal_err)?;

        if updated {
            let body = serde_json::json!({ "updated": true, "id": p.id }).to_string();
            Ok(CallToolResult::success(vec![Content::text(body)]))
        } else {
            tool_err(format!("no memory found with id {}", p.id))
        }
    }
}

// ServerHandler implementation -----------------------------------------------

#[tool_handler]
impl ServerHandler for MemServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("memlite-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Local agent memory store backed by libsql vector search. \
                 Tools: memory_store, memory_search, memory_list, memory_delete, memory_update. \
                 All embeddings are generated server-side; pass plain text only."
                    .to_string(),
            )
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        Ok(self.get_info())
    }
}
