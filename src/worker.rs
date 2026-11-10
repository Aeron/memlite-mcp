use std::{
    collections::HashMap,
    fs::{DirBuilder, File, OpenOptions},
    io::ErrorKind,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, Weak},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use rmcp::{Peer, RoleClient, ServiceExt, service::RunningService};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::{Notify, OnceCell, RwLock},
};

use crate::{
    config::{Config, DEFAULT_EMBEDDING_MAX_BATCH_SIZE, EmbeddingModel},
    db::MemoryStore,
    embedding::EmbeddingService,
    mcp::server::MemServer,
};

const WORKER_PROTOCOL: u32 = 2;
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
const RECONNECT_DELAY: Duration = Duration::from_millis(100);

fn default_embedding_max_batch_size() -> usize {
    DEFAULT_EMBEDDING_MAX_BATCH_SIZE
}

/// Describes one project session without unrelated process configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ProjectSpec {
    project_path: String,
    db_path: PathBuf,
    embedding_model: EmbeddingModel,
    #[serde(default = "default_embedding_max_batch_size")]
    embedding_max_batch_size: usize,
    embedding_cache_dir: PathBuf,
}

impl ProjectSpec {
    /// Builds shared project settings from resolved client configuration.
    pub(crate) fn new(config: &Config, project_path: String) -> Self {
        Self {
            db_path: config.db_path_for(&project_path),
            project_path,
            embedding_model: config.embedding_model,
            embedding_max_batch_size: config.embedding_max_batch_size,
            embedding_cache_dir: config.embedding_cache_dir(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Handshake {
    protocol: u32,
    project: ProjectSpec,
}

#[derive(Debug, Deserialize)]
struct HandshakeEnvelope {
    protocol: u32,
    project: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
struct HandshakeResponse {
    error: Option<String>,
}

/// Owns one lazy database and its shared embedding service.
pub(crate) struct ProjectHandle {
    store: OnceCell<MemoryStore>,
    embedding: Arc<SharedEmbedding>,
    spec: ProjectSpec,
}

impl ProjectHandle {
    /// Opens or returns shared memory storage.
    pub(crate) async fn open_store(&self) -> Result<&MemoryStore> {
        self.store
            .get_or_try_init(|| async {
                let model = self.spec.embedding_model;
                let fastembed_model = model.into();
                let dimensions = fastembed::TextEmbedding::get_model_info(&fastembed_model)
                    .context("configured embedding model is not supported")?
                    .dim;
                MemoryStore::open(&self.spec.db_path, &model.to_string(), dimensions)
                    .await
                    .with_context(|| {
                        format!("opening memory store {}", self.spec.db_path.display())
                    })
            })
            .await
    }

    /// Returns shared storage only when its database already exists.
    pub(crate) async fn store_if_present(&self) -> Result<Option<&MemoryStore>> {
        if let Some(store) = self.store.get() {
            return Ok(Some(store));
        }
        if !self.spec.db_path.exists() {
            return Ok(None);
        }
        Ok(Some(self.open_store().await?))
    }

    /// Returns shared lazy embedding inference.
    pub(crate) fn embedding(&self) -> &EmbeddingService {
        &self.embedding.service
    }

    fn ensure_compatible(&self, requested: &ProjectSpec) -> Result<()> {
        if &self.spec == requested {
            return Ok(());
        }
        bail!(
            "project already open with different shared settings: {}",
            self.spec.db_path.display()
        )
    }
}

struct SharedEmbedding {
    service: EmbeddingService,
    cache_dir: PathBuf,
}

impl SharedEmbedding {
    fn compatible(&self, cache_dir: &Path, model: EmbeddingModel, batch_size: usize) -> bool {
        self.cache_dir == cache_dir
            && self.service.batch_size() == model.max_batch_size().min(batch_size)
    }
}

/// Shares project and model resources inside elected worker.
#[derive(Default)]
pub(crate) struct Registry {
    projects: StdMutex<HashMap<PathBuf, Weak<ProjectHandle>>>,
    models: StdMutex<HashMap<EmbeddingModel, Weak<SharedEmbedding>>>,
}

impl Registry {
    /// Opens or reuses one project resource set.
    pub(crate) fn project(&self, spec: &ProjectSpec) -> Result<Arc<ProjectHandle>> {
        if spec.embedding_max_batch_size == 0 {
            bail!("embedding max batch size must be greater than zero");
        }
        let mut projects = self
            .projects
            .lock()
            .expect("project registry mutex poisoned");
        if let Some(project) = projects.get(&spec.db_path).and_then(Weak::upgrade) {
            project.ensure_compatible(spec)?;
            return Ok(project);
        }

        let project = Arc::new(ProjectHandle {
            store: OnceCell::new(),
            embedding: self.embedding(spec)?,
            spec: spec.clone(),
        });
        projects.insert(spec.db_path.clone(), Arc::downgrade(&project));

        tracing::info!(
            project = %spec.project_path,
            db = %spec.db_path.display(),
            model = %spec.embedding_model,
            "shared project opened"
        );
        Ok(project)
    }

    fn embedding(&self, spec: &ProjectSpec) -> Result<Arc<SharedEmbedding>> {
        let mut models = self.models.lock().expect("model registry mutex poisoned");
        if let Some(shared) = models.get(&spec.embedding_model).and_then(Weak::upgrade) {
            if !shared.compatible(
                &spec.embedding_cache_dir,
                spec.embedding_model,
                spec.embedding_max_batch_size,
            ) {
                bail!(
                    "embedding model '{}' already loaded with different cache or batch settings",
                    spec.embedding_model
                );
            }
            return Ok(shared);
        }

        let shared = Arc::new(SharedEmbedding {
            service: EmbeddingService::new(
                spec.embedding_model,
                spec.embedding_cache_dir.clone(),
                spec.embedding_max_batch_size,
            ),
            cache_dir: spec.embedding_cache_dir.clone(),
        });
        models.insert(spec.embedding_model, Arc::downgrade(&shared));
        Ok(shared)
    }
}

struct WorkerPaths {
    dir: PathBuf,
    socket: PathBuf,
    lock: PathBuf,
}

impl WorkerPaths {
    fn new(dir: PathBuf) -> Self {
        let hash = format!("{:x}", Sha256::digest(dir.as_os_str().as_encoded_bytes()));
        Self {
            socket: PathBuf::from("/tmp")
                .join(format!("memlite-{}", &hash[..32]))
                .join("worker.sock"),
            lock: dir.join("worker.lock"),
            dir,
        }
    }

    fn prepare_socket_dir(&self) -> Result<()> {
        let dir = self.socket.parent().expect("worker socket has a parent");
        match DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("creating shared worker socket directory {}", dir.display())
                });
            }
        }

        // Rejecting pre-created paths instead of repairing a possibly hostile directory
        let metadata = std::fs::symlink_metadata(dir)?;
        if !metadata.is_dir()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o777 != 0o700
        {
            bail!(
                "refusing untrusted shared worker socket directory {}: expected a directory owned by the current user with mode 0700",
                dir.display()
            );
        }
        Ok(())
    }
}

struct WorkerGuard {
    _file: File,
}

impl WorkerGuard {
    fn try_acquire(path: &Path) -> Result<Option<Self>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        match file.try_lock().map_err(std::io::Error::from) {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

type ClientService = RunningService<RoleClient, ()>;

enum ConnectAttempt {
    Connected(ClientService),
    Rejected(String),
}

/// Maintains one reconnecting MCP client to elected worker.
pub(crate) struct Remote {
    paths: WorkerPaths,
    project: ProjectSpec,
    peer: RwLock<Option<Peer<RoleClient>>>,
    failure: RwLock<Option<String>>,
    changed: Notify,
}

impl Remote {
    /// Connects to current worker, electing this process when needed.
    pub(crate) async fn connect(db_dir: PathBuf, project: ProjectSpec) -> Result<Arc<Self>> {
        let remote = Arc::new(Self {
            paths: WorkerPaths::new(db_dir),
            project,
            peer: RwLock::new(None),
            failure: RwLock::new(None),
            changed: Notify::new(),
        });
        let service = remote.connect_or_elect().await?;
        remote.install(&service).await;
        tokio::spawn(Arc::clone(&remote).monitor(service));
        Ok(remote)
    }

    /// Returns current worker peer, waiting briefly through promotion.
    pub(crate) async fn peer(&self) -> Result<Peer<RoleClient>> {
        tokio::time::timeout(CONNECT_TIMEOUT, async {
            loop {
                let notified = self.changed.notified();
                if let Some(peer) = self.peer.read().await.clone() {
                    return Ok(peer);
                }
                if let Some(error) = self.failure.read().await.clone() {
                    return Err(anyhow!(error));
                }
                notified.await;
            }
        })
        .await
        .map_err(|_| anyhow!("shared worker unavailable after {CONNECT_TIMEOUT:?}"))?
    }

    async fn install(&self, service: &ClientService) {
        *self.failure.write().await = None;
        *self.peer.write().await = Some(service.peer().clone());
        self.changed.notify_waiters();
    }

    async fn monitor(self: Arc<Self>, mut service: ClientService) {
        loop {
            let result = service.waiting().await;
            *self.peer.write().await = None;
            self.changed.notify_waiters();
            tracing::warn!(?result, "shared worker connection closed; re-electing");

            loop {
                match self.connect_or_elect().await {
                    Ok(next) => {
                        service = next;
                        self.install(&service).await;
                        break;
                    }
                    Err(error) => {
                        *self.failure.write().await = Some(format!("{error:#}"));
                        self.changed.notify_waiters();
                        tracing::warn!(?error, "shared worker reconnect failed");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }

    async fn connect_or_elect(&self) -> Result<ClientService> {
        self.paths.prepare_socket_dir()?;
        std::fs::create_dir_all(&self.paths.dir)?;
        let started = Instant::now();
        let mut last_error: String;

        loop {
            match tokio::time::timeout(CONNECT_ATTEMPT_TIMEOUT, self.connect_once()).await {
                Ok(Ok(ConnectAttempt::Connected(service))) => {
                    tracing::info!(
                        project = %self.project.project_path,
                        "connected to shared worker"
                    );
                    return Ok(service);
                }
                Ok(Ok(ConnectAttempt::Rejected(error))) => {
                    bail!("shared worker rejected project: {error}");
                }
                Ok(Err(error)) => last_error = format!("{error:#}"),
                Err(_) => {
                    last_error = format!("connection attempt exceeded {CONNECT_ATTEMPT_TIMEOUT:?}");
                }
            }

            if let Some(guard) = WorkerGuard::try_acquire(&self.paths.lock)? {
                start_worker(&self.paths, guard).await?;
                continue;
            }
            if started.elapsed() >= CONNECT_TIMEOUT {
                return Err(anyhow!(
                    "timed out connecting to shared worker at {}: {}",
                    self.paths.socket.display(),
                    last_error
                ));
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    }

    async fn connect_once(&self) -> Result<ConnectAttempt> {
        let mut stream = UnixStream::connect(&self.paths.socket).await?;
        write_frame(
            &mut stream,
            &Handshake {
                protocol: WORKER_PROTOCOL,
                project: self.project.clone(),
            },
        )
        .await?;
        let response: HandshakeResponse = read_frame(&mut stream).await?;
        if let Some(error) = response.error {
            return Ok(ConnectAttempt::Rejected(error));
        }
        Ok(ConnectAttempt::Connected(().serve(stream).await?))
    }
}

async fn start_worker(paths: &WorkerPaths, guard: WorkerGuard) -> Result<()> {
    match std::fs::symlink_metadata(&paths.socket) {
        Ok(metadata) if metadata.file_type().is_socket() => std::fs::remove_file(&paths.socket)?,
        Ok(_) => bail!(
            "refusing to replace non-socket worker path: {}",
            paths.socket.display()
        ),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let listener = UnixListener::bind(&paths.socket)
        .with_context(|| format!("binding shared worker socket {}", paths.socket.display()))?;
    std::fs::set_permissions(&paths.socket, std::fs::Permissions::from_mode(0o600))?;
    let registry = Arc::new(Registry::default());
    tracing::info!(socket = %paths.socket.display(), "shared worker elected");

    tokio::spawn(async move {
        let _guard = guard;
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let registry = Arc::clone(&registry);
                    tokio::spawn(async move {
                        if let Err(error) = serve_connection(stream, registry).await {
                            tracing::warn!(?error, "shared worker session failed");
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "shared worker accept failed; retrying");
                    tokio::time::sleep(RECONNECT_DELAY).await;
                }
            }
        }
    });
    Ok(())
}

async fn serve_connection(mut stream: UnixStream, registry: Arc<Registry>) -> Result<()> {
    let handshake: HandshakeEnvelope = read_frame(&mut stream).await?;
    if handshake.protocol != WORKER_PROTOCOL {
        write_frame(
            &mut stream,
            &HandshakeResponse {
                error: Some(format!(
                    "protocol mismatch: worker {WORKER_PROTOCOL}, client {}",
                    handshake.protocol
                )),
            },
        )
        .await?;
        return Ok(());
    }

    let project_spec: ProjectSpec = serde_json::from_value(handshake.project)?;
    let project = match registry.project(&project_spec) {
        Ok(project) => project,
        Err(error) => {
            write_frame(
                &mut stream,
                &HandshakeResponse {
                    error: Some(format!("{error:#}")),
                },
            )
            .await?;
            return Ok(());
        }
    };
    write_frame(&mut stream, &HandshakeResponse { error: None }).await?;

    let service = MemServer::new(project).serve(stream).await?;
    let result = service.waiting().await;
    tracing::debug!(?result, "shared worker session stopped");
    Ok(())
}

async fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.len() > MAX_HANDSHAKE_BYTES {
        bail!("shared worker handshake is too large");
    }
    stream.write_u32(body.len() as u32).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

async fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    let len = stream.read_u32().await? as usize;
    if len == 0 || len > MAX_HANDSHAKE_BYTES {
        bail!("invalid shared worker handshake length: {len}");
    }
    let mut body = vec![0; len];
    stream.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body)?)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn worker_lock_has_one_owner_and_promotes_after_drop() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("worker.lock");
        let first = WorkerGuard::try_acquire(&path).unwrap().unwrap();
        assert!(WorkerGuard::try_acquire(&path).unwrap().is_none());
        drop(first);
        assert!(WorkerGuard::try_acquire(&path).unwrap().is_some());
    }

    #[test]
    fn socket_directory_is_private_and_reusable() {
        let db_dir = tempdir().unwrap();
        let paths = WorkerPaths::new(db_dir.path().to_path_buf());
        let dir = paths.socket.parent().unwrap();
        paths.prepare_socket_dir().unwrap();
        paths.prepare_socket_dir().unwrap();

        let metadata = std::fs::symlink_metadata(dir).unwrap();
        assert!(metadata.is_dir());
        assert_eq!(metadata.mode() & 0o777, 0o700);
        assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
        std::fs::remove_dir(dir).unwrap();
    }

    #[tokio::test]
    async fn connection_rejects_permissive_socket_directory_before_handshake() {
        for mode in [0o755, 0o770, 0o707] {
            let project = tempdir().unwrap();
            let db_dir = tempdir().unwrap();
            let paths = WorkerPaths::new(db_dir.path().to_path_buf());
            let dir = paths.socket.parent().unwrap();
            std::fs::create_dir(dir).unwrap();
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap();
            let listener = UnixListener::bind(&paths.socket).unwrap();

            let error = Remote::connect(
                db_dir.path().to_path_buf(),
                spec(project.path(), db_dir.path()),
            )
            .await
            .err()
            .expect("untrusted socket directory must be rejected");

            assert!(
                error
                    .to_string()
                    .contains("untrusted shared worker socket directory")
            );
            assert_eq!(std::fs::metadata(dir).unwrap().mode() & 0o777, mode);
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[tokio::test]
    async fn connection_rejects_symlinked_socket_directory_before_handshake() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let target = tempdir().unwrap();
        let paths = WorkerPaths::new(db_dir.path().to_path_buf());
        let dir = paths.socket.parent().unwrap();
        std::os::unix::fs::symlink(target.path(), dir).unwrap();
        let listener = UnixListener::bind(&paths.socket).unwrap();

        let error = Remote::connect(
            db_dir.path().to_path_buf(),
            spec(project.path(), db_dir.path()),
        )
        .await
        .err()
        .expect("symlinked socket directory must be rejected");

        assert!(
            error
                .to_string()
                .contains("untrusted shared worker socket directory")
        );
        assert!(std::fs::symlink_metadata(dir).unwrap().is_symlink());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
        std::fs::remove_file(dir).unwrap();
    }

    #[test]
    fn socket_directory_rejects_files_and_dangling_symlinks() {
        let db_dir = tempdir().unwrap();
        let mut paths = WorkerPaths::new(db_dir.path().to_path_buf());
        paths.socket = db_dir.path().join("socket-dir/worker.sock");
        let dir = paths.socket.parent().unwrap();
        std::fs::write(dir, b"").unwrap();
        assert!(paths.prepare_socket_dir().is_err());
        std::fs::remove_file(dir).unwrap();

        std::os::unix::fs::symlink(db_dir.path().join("missing"), dir).unwrap();
        assert!(paths.prepare_socket_dir().is_err());
    }

    #[tokio::test]
    async fn previous_protocol_without_batch_field_gets_mismatch_response() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let mut request = serde_json::to_value(Handshake {
            protocol: WORKER_PROTOCOL - 1,
            project: spec(project.path(), db_dir.path()),
        })
        .unwrap();
        request["project"]
            .as_object_mut()
            .unwrap()
            .remove("embedding_max_batch_size");
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::default());
        let serving = tokio::spawn(serve_connection(server, registry));

        write_frame(&mut client, &request).await.unwrap();
        let response: HandshakeResponse = read_frame(&mut client).await.unwrap();
        let expected = format!(
            "protocol mismatch: worker {WORKER_PROTOCOL}, client {}",
            WORKER_PROTOCOL - 1
        );

        assert_eq!(response.error.as_deref(), Some(expected.as_str()));
        serving.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn wrong_protocol_with_empty_project_gets_mismatch_response() {
        let request = serde_json::json!({
            "protocol": WORKER_PROTOCOL + 1,
            "project": {},
        });
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::default());
        let serving = tokio::spawn(serve_connection(server, Arc::clone(&registry)));

        write_frame(&mut client, &request).await.unwrap();
        let response: HandshakeResponse = read_frame(&mut client).await.unwrap();
        let expected = format!(
            "protocol mismatch: worker {WORKER_PROTOCOL}, client {}",
            WORKER_PROTOCOL + 1
        );

        assert_eq!(response.error.as_deref(), Some(expected.as_str()));
        serving.await.unwrap().unwrap();
        assert!(registry.projects.lock().unwrap().is_empty());
        assert!(registry.models.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn handshake_rejects_zero_embedding_batch_before_registering_resources() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let mut project_spec = spec(project.path(), db_dir.path());
        project_spec.embedding_max_batch_size = 0;
        let request = Handshake {
            protocol: WORKER_PROTOCOL,
            project: project_spec,
        };
        let (mut client, server) = UnixStream::pair().unwrap();
        let registry = Arc::new(Registry::default());
        let serving = tokio::spawn(serve_connection(server, Arc::clone(&registry)));

        write_frame(&mut client, &request).await.unwrap();
        let response: HandshakeResponse = read_frame(&mut client).await.unwrap();

        assert_eq!(
            response.error.as_deref(),
            Some("embedding max batch size must be greater than zero")
        );
        serving.await.unwrap().unwrap();
        assert!(registry.projects.lock().unwrap().is_empty());
        assert!(registry.models.lock().unwrap().is_empty());
        assert!(!request.project.db_path.exists());
        assert!(!request.project.embedding_cache_dir.exists());
    }

    #[test]
    fn registry_reuses_project_handle_without_creating_database() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let spec = spec(project.path(), db_dir.path());
        let registry = Registry::default();
        let first = registry.project(&spec).unwrap();
        let second = registry.project(&spec).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!spec.db_path.exists());
    }

    #[tokio::test]
    async fn election_replaces_unresponsive_socket_when_lock_is_available() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let paths = WorkerPaths::new(db_dir.path().to_path_buf());
        paths.prepare_socket_dir().unwrap();

        let listener = UnixListener::bind(&paths.socket).unwrap();
        let stalled = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let remote = tokio::time::timeout(
            Duration::from_secs(5),
            Remote::connect(
                db_dir.path().to_path_buf(),
                spec(project.path(), db_dir.path()),
            ),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(remote.peer().await.is_ok());
        stalled.abort();
        std::fs::remove_dir_all(paths.socket.parent().unwrap()).unwrap();
    }

    #[test]
    fn registry_shares_each_embedding_model_once() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let small = spec(project.path(), db_dir.path());
        let mut m3 = small.clone();
        m3.embedding_model = EmbeddingModel::M3;
        let registry = Registry::default();

        let first = registry.embedding(&small).unwrap();
        let second = registry.embedding(&small).unwrap();
        let other = registry.embedding(&m3).unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn registry_rejects_different_cache_for_active_model() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let first = spec(project.path(), db_dir.path());
        let mut conflicting = first.clone();
        conflicting.embedding_cache_dir = db_dir.path().join("other-cache");
        let registry = Registry::default();

        let _active = registry.embedding(&first).unwrap();
        let error = registry.embedding(&conflicting).err().unwrap();

        assert!(
            error
                .to_string()
                .contains("different cache or batch settings")
        );
    }

    #[test]
    fn registry_rejects_different_batch_for_active_model() {
        let project = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let first = spec(project.path(), db_dir.path());
        let mut conflicting = first.clone();
        conflicting.embedding_max_batch_size = 16;
        let registry = Registry::default();

        let _active = registry.embedding(&first).unwrap();
        let error = registry.embedding(&conflicting).err().unwrap();

        assert!(
            error
                .to_string()
                .contains("different cache or batch settings")
        );
    }

    fn spec(project: &Path, db_dir: &Path) -> ProjectSpec {
        ProjectSpec {
            project_path: project.to_string_lossy().into_owned(),
            db_path: db_dir.join("project.db"),
            embedding_model: EmbeddingModel::Small,
            embedding_max_batch_size: DEFAULT_EMBEDDING_MAX_BATCH_SIZE,
            embedding_cache_dir: db_dir.join("cache"),
        }
    }
}
