use std::{sync::Arc, time::Duration};

use anyhow::anyhow;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceError, ServiceExt,
    model::*,
    service::{NotificationContext, QuitReason, RequestContext},
};
use tokio::sync::{Notify, OnceCell};

use super::server::server_info;
use crate::{
    config::Config,
    worker::{ProjectSpec, Remote},
};

/// Serves one editor client while delegating memory work to elected worker.
pub struct ProxyServer {
    config: Config,
    upstream: OnceCell<Arc<Remote>>,
    ready: Notify,
    shutdown: Arc<Notify>,
}

impl ProxyServer {
    /// Creates one stdio proxy session.
    pub fn new(config: Config) -> Self {
        Self {
            config,
            upstream: OnceCell::new(),
            ready: Notify::new(),
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Drives MCP proxy to completion over stdio.
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
                    Ok(QuitReason::JoinError(error)) => {
                        tracing::warn!(%error, "server send task failed");
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "server loop panicked");
                    }
                }
                result.map(|_| ()).map_err(anyhow::Error::from)
            }
            _ = shutdown.notified() => {
                tokio::task::yield_now().await;
                Err(anyhow!("exiting: initialization failed (see logs)"))
            }
        }
    }

    async fn upstream(&self) -> Result<&Arc<Remote>, McpError> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let notified = self.ready.notified();
                if let Some(upstream) = self.upstream.get() {
                    return upstream;
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| {
            McpError::internal_error("shared worker unavailable after 10s".to_owned(), None)
        })
    }
}

impl ServerHandler for ProxyServer {
    fn get_info(&self) -> ServerInfo {
        server_info()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let peer = self
            .upstream()
            .await?
            .peer()
            .await
            .map_err(internal_error)?;
        peer.list_tools(request).await.map_err(forward_error)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let peer = self
            .upstream()
            .await?
            .peer()
            .await
            .map_err(internal_error)?;
        peer.call_tool(request).await.map_err(forward_error)
    }

    #[allow(deprecated)]
    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if self.upstream.get().is_some() {
            return;
        }

        let peer = context.peer.peer_info();
        let supports_roots = peer.is_some_and(|info| info.capabilities.roots.is_some());
        const LIST_ROOTS_TIMEOUT: Duration = Duration::from_secs(10);
        let root_uris = if self.config.explicit_project_path().is_some() {
            vec![]
        } else if supports_roots {
            match tokio::time::timeout(LIST_ROOTS_TIMEOUT, context.peer.list_roots()).await {
                Ok(Ok(result)) => result.roots.into_iter().map(|root| root.uri).collect(),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "list_roots failed");
                    vec![]
                }
                Err(_) => {
                    tracing::warn!("list_roots timed out; falling back to CWD");
                    vec![]
                }
            }
        } else {
            vec![]
        };

        let project_path = match self.config.resolve_project_path(&root_uris) {
            Ok(path) => path,
            Err(error) => {
                self.fail_initialization(&context, error.to_string()).await;
                return;
            }
        };
        let db_dir = match self.config.prepare_db_dir() {
            Ok(path) => path,
            Err(error) => {
                self.fail_initialization(&context, error.to_string()).await;
                return;
            }
        };
        let project = ProjectSpec::new(&self.config, project_path);
        match Remote::connect(db_dir, project).await {
            Ok(upstream) => {
                let _ = self.upstream.set(upstream);
                self.ready.notify_waiters();
            }
            Err(error) => {
                self.fail_initialization(&context, error.to_string()).await;
            }
        }
    }
}

impl ProxyServer {
    #[allow(deprecated)]
    async fn fail_initialization(&self, context: &NotificationContext<RoleServer>, error: String) {
        tracing::error!(%error, "fatal initialization error");
        let _ = context
            .peer
            .notify_logging_message(LoggingMessageNotificationParam::new(
                LoggingLevel::Error,
                serde_json::json!(error),
            ))
            .await;
        self.ready.notify_waiters();
        self.shutdown.notify_one();
    }
}

fn internal_error(error: anyhow::Error) -> McpError {
    McpError::internal_error(error.to_string(), None)
}

fn forward_error(error: ServiceError) -> McpError {
    match error {
        ServiceError::McpError(error) => error,
        other => McpError::internal_error(other.to_string(), None),
    }
}
