use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    serde_json,
    service::NotificationContext,
    tool, tool_handler, tool_router,
};
use tokio::sync::OnceCell;

const INSTRUCTIONS: &str = include_str!("instructions.md");

use super::params::{
    DeleteParams, ListParams, SearchParams, StoreParams, TagsParams, UpdateParams,
};
use crate::{config::Config, db::MemoryStore, embedding::EmbeddingService};

#[derive(Clone)]
pub struct MemServer {
    db_path: Arc<OnceLock<Option<PathBuf>>>,
    store: Arc<OnceCell<MemoryStore>>,
    embedding: EmbeddingService,
    config: Arc<Config>,

    // Used by the #[tool_handler] macro-generated call_tool / list_tools impls
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
            tool_router: Self::tool_router(),
        }
    }

    // Opens (or returns cached) store, creating the DB file if needed.
    async fn open_store(&self) -> Result<&MemoryStore, McpError> {
        let db_path = self.db_path.get().ok_or_else(|| {
            McpError::internal_error(
                "store unavailable: project path could not be determined; \
                 set --project-path or MEMLITE_PROJECT_PATH",
                None,
            )
        })?;
        self.store
            .get_or_try_init(|| {
                let path = db_path.clone();
                async move {
                    MemoryStore::open(path.as_deref())
                        .await
                        .map_err(internal_err)
                }
            })
            .await
    }

    // Returns Some(store) when the DB file exists, None when it does not (for read-only tools).
    async fn store_if_present(&self) -> Result<Option<&MemoryStore>, McpError> {
        if let Some(store) = self.store.get() {
            return Ok(Some(store));
        }
        let db_path = self.db_path.get().ok_or_else(|| {
            McpError::internal_error(
                "store unavailable: project path could not be determined; \
                 set --project-path or MEMLITE_PROJECT_PATH",
                None,
            )
        })?;
        if let Some(path) = db_path
            && !path.exists()
        {
            return Ok(None);
        }
        Ok(Some(self.open_store().await?))
    }

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
            .map_err(internal_err)?;
        let tags = p.tags.unwrap_or_default();
        let store = self.open_store().await?;
        let id = store
            .insert(
                &p.content,
                p.summary.as_deref(),
                &tags,
                &self.config.agent,
                &embedding,
            )
            .await
            .map_err(internal_err)?;

        let body = serde_json::json!({ "id": id }).to_string();
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

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
        let embedding = self.embedding.embed(p.query).await.map_err(internal_err)?;
        let results = store
            .search(
                &embedding,
                self.config.agent_filter(),
                p.tags.as_deref(),
                limit,
            )
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
                    "agent":      m.agent,
                    "created_at": m.created_at,
                    "updated_at": m.updated_at,
                    "distance":   dist,
                })
            })
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&body).unwrap(),
        )]))
    }

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
            .map_err(internal_err)?;

        let body = serde_json::to_string(&memories).unwrap();
        Ok(CallToolResult::success(vec![Content::text(body)]))
    }

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
            .map_err(internal_err)?;

        let body: Vec<_> = tags
            .into_iter()
            .map(|(tag, count)| serde_json::json!({ "tag": tag, "count": count }))
            .collect();
        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string(&body).unwrap(),
        )]))
    }

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
        let deleted = store.delete(p.id).await.map_err(internal_err)?;
        if deleted {
            let body = serde_json::json!({ "deleted": true, "id": p.id }).to_string();
            Ok(CallToolResult::success(vec![Content::text(body)]))
        } else {
            tool_err(format!("no memory found with id {}", p.id))
        }
    }

    #[tool(
        description = "Update an existing memory. Provide id plus any fields to change. Updating content triggers re-embedding. Pass summary as empty string to clear it."
    )]
    async fn memory_update(
        &self,
        Parameters(p): Parameters<UpdateParams>,
    ) -> Result<CallToolResult, McpError> {
        if p.content.is_none() && p.summary.is_none() && p.tags.is_none() {
            return tool_err("at least one of content, summary, or tags must be provided");
        }

        let Some(store) = self.store_if_present().await? else {
            return tool_err(format!("no memory found with id {}", p.id));
        };
        let new_embedding = if let Some(ref c) = p.content {
            Some(
                self.embedding
                    .embed(c.clone())
                    .await
                    .map_err(internal_err)?,
            )
        } else {
            None
        };

        let updated = store
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
        let Some(project_path) =
            Config::resolve_project_path(self.config.explicit_project_path(), &root_uris)
        else {
            tracing::error!(
                "cannot determine project path: set --project-path or MEMLITE_PROJECT_PATH"
            );
            return;
        };

        let db_path = self.config.db_path_for(&project_path);
        tracing::info!(path = ?db_path, project = %project_path, "resolved database path");
        if self.db_path.set(db_path).is_err() {
            tracing::warn!("db path already set");
        }
    }
}

fn tool_err(msg: impl std::fmt::Display) -> Result<CallToolResult, McpError> {
    let mut result = CallToolResult::success(vec![Content::text(msg.to_string())]);
    result.is_error = Some(true);
    Ok(result)
}

fn internal_err(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}
