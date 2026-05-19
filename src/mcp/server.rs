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
    DeleteParams, ListParams, SearchParams, StoreParams, TagsParams, UpdateParams,
};
use crate::{
    config::Config,
    db::{InsertOptions, MemoryStore, UpdateOptions},
    embedding::EmbeddingService,
};

const INSTRUCTIONS: &str = include_str!("instructions.md");

#[derive(Clone)]
pub struct MemServer {
    db_path: Arc<OnceLock<Option<PathBuf>>>,
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
        tracing::info!("memlite-mcp ready");

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
    async fn open_store(&self) -> Result<&MemoryStore, McpError> {
        let db_path = self.db_path.get().ok_or_else(|| self.path_not_resolved())?;
        self.store
            .get_or_try_init(|| {
                let path = db_path.clone();
                async move {
                    MemoryStore::open(path.as_deref())
                        .await
                        .map_err(|e| McpError::from(ServerError::StoreOpen(e)))
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
        if let Some(path) = db_path
            && !path.exists()
        {
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
        let embedding = self
            .embedding
            .embed(p.content.clone())
            .await
            .map_err(ServerError::Embed)?;
        let tags = p.tags.unwrap_or_default();
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
                },
            )
            .await
            .map_err(ServerError::StoreOp)?;

        let body = serde_json::json!({ "id": id }).to_string();
        Ok(CallToolResult::success(vec![Content::text(body)]))
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
                self.config.agent_filter(),
                p.tags.as_deref(),
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
        let limit = p.limit.unwrap_or(20).min(100);
        let offset = p.offset.unwrap_or(0);
        let Some(store) = self.store_if_present().await? else {
            return Ok(CallToolResult::success(vec![Content::text("[]")]));
        };
        let memories = store
            .list(self.config.agent_filter(), p.tags.as_deref(), limit, offset)
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
                    "last_seen_at":    m.last_seen_at,
                    "effective_confidence": m.effective_confidence(),
                    "age_days":             m.age_days(),
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
        let deleted = store.delete(p.id).await.map_err(ServerError::StoreOp)?;
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
        {
            return tool_err(
                "at least one of content, summary, tags, context, confidence, or decay_rate must be provided",
            );
        }

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
                },
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

        let root_uris = if supports_roots {
            match context.peer.list_roots().await {
                Ok(result) => result.roots.into_iter().map(|r| r.uri).collect::<Vec<_>>(),
                Err(e) => {
                    tracing::warn!("list_roots failed: {e}");
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
