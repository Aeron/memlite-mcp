use std::{
    fs::OpenOptions,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Result, anyhow, bail};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use strum::Display;
use tracing_subscriber::{EnvFilter, fmt};

pub(crate) const DEFAULT_EMBEDDING_MAX_BATCH_SIZE: usize = 128;

/// Holds shared project, storage, embedding, and logging options.
#[derive(Args)]
pub struct Config {
    /// Project path used as the shared database identity. Equivalent relative paths
    /// resolve to an absolute form before hashing. Defaults to the first workspace root
    /// reported by the MCP client, or CWD if none is available.
    #[arg(long = "project-path", env = "MEMLITE_PROJECT_PATH", global = true)]
    project_path: Option<String>,

    /// Directory for per-project DB files. Defaults to the XDG data home.
    #[arg(long = "db-dir", env = "MEMLITE_DB_DIR", global = true)]
    db_dir: Option<PathBuf>,

    /// Allows the filesystem root ("/") as the project path.
    /// Off by default; using the whole filesystem as a project is almost certainly wrong.
    #[arg(long = "allow-root", env = "MEMLITE_ALLOW_ROOT", default_value_t = false, value_parser = clap::builder::BoolishValueParser::new(), global = true)]
    allow_root: bool,

    /// Allows the home directory as the project path.
    /// Off by default; the home directory spans many unrelated projects.
    #[arg(long = "allow-home", env = "MEMLITE_ALLOW_HOME", default_value_t = false, value_parser = clap::builder::BoolishValueParser::new(), global = true)]
    allow_home: bool,

    /// Embedding model to use for new memories.
    #[arg(
        long = "embedding-model",
        env = "MEMLITE_EMBEDDING_MODEL",
        default_value_t = EmbeddingModel::Small,
        global = true
    )]
    pub embedding_model: EmbeddingModel,

    /// Maximum requested embedding inference batch size.
    #[arg(
        long = "embedding-max-batch-size",
        env = "MEMLITE_EMBEDDING_MAX_BATCH_SIZE",
        default_value_t = DEFAULT_EMBEDDING_MAX_BATCH_SIZE,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..),
        global = true
    )]
    pub embedding_max_batch_size: usize,

    /// Directory for cached embedding model files. Defaults to `<cache-dir>/memlite-mcp`.
    #[arg(
        long = "embedding-cache-dir",
        env = "MEMLITE_EMBEDDING_CACHE_DIR",
        global = true
    )]
    embedding_cache_dir: Option<PathBuf>,

    /// Write logs to this file instead of stderr. Useful when the MCP host does not
    /// surface the server's stderr (e.g. Zed has no MCP log panel).
    #[arg(long = "log-file", env = "MEMLITE_LOG_FILE", global = true)]
    pub log_file: Option<PathBuf>,

    /// Minimum log level when `RUST_LOG` is not set. Applies to both file and stderr output.
    #[arg(long = "log-level", env = "MEMLITE_LOG_LEVEL", default_value_t = LogLevel::Info, global = true)]
    log_level: LogLevel,
}

/// Minimum log verbosity level, used when `RUST_LOG` is not set.
#[derive(Debug, Clone, Copy, ValueEnum, Display)]
#[strum(serialize_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Supported local embedding models.
#[derive(Debug, Clone, Copy, Deserialize, Eq, Hash, PartialEq, Serialize, ValueEnum)]
pub enum EmbeddingModel {
    /// BAAI/bge-small-en-v1.5 (384 dims, ~67 MB; default)
    #[value(name = "bge-small-en-v1.5")]
    Small,
    /// BAAI/bge-base-en-v1.5 (768 dims, ~210 MB)
    #[value(name = "bge-base-en-v1.5")]
    Base,
    /// BAAI/bge-large-en-v1.5 (1024 dims, ~620 MB)
    #[value(name = "bge-large-en-v1.5")]
    Large,
    /// BAAI/bge-m3 (1024 dims, ~2.3 GB; multilingual, 8192-token context)
    #[value(name = "bge-m3")]
    M3,
}

impl EmbeddingModel {
    /// Returns conservative inference batch limit.
    pub(crate) const fn max_batch_size(self) -> usize {
        match self {
            EmbeddingModel::Small => 128,
            EmbeddingModel::Base => 64,
            EmbeddingModel::Large => 32,
            EmbeddingModel::M3 => 8,
        }
    }
}

impl std::fmt::Display for EmbeddingModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.to_possible_value()
            .expect("all variants covered")
            .get_name()
            .fmt(f)
    }
}

impl From<EmbeddingModel> for fastembed::EmbeddingModel {
    fn from(m: EmbeddingModel) -> Self {
        match m {
            EmbeddingModel::Small => fastembed::EmbeddingModel::BGESmallENV15,
            EmbeddingModel::Base => fastembed::EmbeddingModel::BGEBaseENV15,
            EmbeddingModel::Large => fastembed::EmbeddingModel::BGELargeENV15,
            EmbeddingModel::M3 => fastembed::EmbeddingModel::BGEM3,
        }
    }
}

impl Config {
    /// Returns the resolved embedding model cache directory.
    pub fn embedding_cache_dir(&self) -> PathBuf {
        let path = match &self.embedding_cache_dir {
            Some(path) => path.clone(),
            None => dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("memlite-mcp"),
        };
        absolute_path(path)
    }

    /// Returns the raw CLI/env project path, if provided explicitly.
    pub fn explicit_project_path(&self) -> Option<&str> {
        self.project_path.as_deref()
    }

    /// Returns absolute shared database directory.
    pub fn db_dir(&self) -> PathBuf {
        let path = self.db_dir.clone().unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("memlite-mcp")
        });
        let path = absolute_path(path);
        std::fs::canonicalize(&path).unwrap_or(path)
    }

    /// Creates and returns canonical shared database directory.
    pub fn prepare_db_dir(&self) -> Result<PathBuf> {
        let path = self.db_dir();
        std::fs::create_dir_all(&path)?;
        Ok(std::fs::canonicalize(path)?)
    }

    /// Returns DB file path for given project path.
    pub fn db_path_for(&self, project_path: &str) -> PathBuf {
        let hash = Sha256::digest(project_path.as_bytes());
        self.db_dir().join(format!("{hash:x}.db"))
    }

    /// Resolves the effective project path from the explicit override or MCP roots.
    ///
    /// Priority: explicit arg > first local file:// root URI > CWD.
    /// Validates the candidate once: rejects the filesystem root unless `--allow-root` is set,
    /// and rejects the home directory unless `--allow-home` is set.
    pub fn resolve_project_path(&self, root_uris: &[String]) -> Result<String> {
        let path = if let Some(p) = self.project_path.as_deref() {
            p.to_owned()
        } else {
            root_uris
                .iter()
                .find_map(|uri| local_file_uri_path(uri))
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(str::to_owned))
                })
                .ok_or_else(|| {
                    anyhow!(
                        "cannot determine project path: set --project-path or MEMLITE_PROJECT_PATH"
                    )
                })?
        };

        let path = std::path::absolute(path)?;
        // Resolving parent segments before validation to keep project identities stable
        let path = if path.components().any(|part| part == Component::ParentDir) {
            std::fs::canonicalize(&path).unwrap_or_else(|_| {
                let mut normalized = PathBuf::new();
                for component in path.components() {
                    if component == Component::ParentDir {
                        normalized.pop();
                    } else {
                        normalized.push(component);
                    }
                }
                normalized
            })
        } else {
            path
        };
        let path = path.to_string_lossy().into_owned();

        if !self.allow_root
            && (path == "/"
                || Path::new(&path)
                    .canonicalize()
                    .is_ok_and(|p| p == Path::new("/")))
        {
            bail!(
                "project path '/' is the filesystem root; use --allow-root to opt in, or set a real project directory via --project-path or MEMLITE_PROJECT_PATH"
            );
        }
        if !self.allow_home && is_home_path(&path) {
            bail!(
                "project path is the home directory; use --allow-home to opt in, or set a specific project directory via --project-path or MEMLITE_PROJECT_PATH"
            );
        }
        Ok(path)
    }

    /// Initialises the global tracing subscriber when a log destination is available.
    ///
    /// Writes to the configured file for every command. Otherwise writes to stderr only
    /// when `log_to_stderr` is true. Respects `RUST_LOG` when set; otherwise uses
    /// `--log-level` (default: info). Must be called once at startup.
    pub fn init_logging(&self, log_to_stderr: bool) -> Result<()> {
        if self.log_file.is_none() && !log_to_stderr {
            return Ok(());
        }

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(self.log_level.to_string()));

        if let Some(log_path) = &self.log_file {
            if let Some(parent) = log_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)?;
            fmt()
                .with_env_filter(filter)
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .init();
        } else {
            fmt()
                .with_env_filter(filter)
                .with_writer(std::io::stderr)
                .init();
        }
        Ok(())
    }
}

fn absolute_path(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

/// Returns true when `path` is the current user's home directory.
///
/// Treats the home directory as too broad for a project; gated by `--allow-home`.
fn is_home_path(path: &str) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let candidate = Path::new(path);
    if candidate == home {
        return true;
    }
    match (candidate.canonicalize(), home.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn local_file_uri_path(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://")?;
    let path = path
        .get(..9)
        .filter(|host| host.eq_ignore_ascii_case("localhost"))
        .map_or(path, |_| &path[9..]);
    if !path.starts_with('/') || path.contains(['?', '#']) {
        return None;
    }

    // Decoding percent escapes as bytes to preserve UTF-8 file names
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = (bytes.next()? as char).to_digit(16)? as u8;
            let low = (bytes.next()? as char).to_digit(16)? as u8;
            decoded.push((high << 4) | low);
        } else {
            decoded.push(byte);
        }
    }
    if decoded.contains(&0) {
        return None;
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(project_path: &str) -> Config {
        Config {
            project_path: Some(project_path.into()),
            db_dir: None,
            allow_root: false,
            allow_home: false,
            embedding_model: EmbeddingModel::Small,
            embedding_max_batch_size: 128,
            embedding_cache_dir: None,
            log_file: None,
            log_level: LogLevel::Info,
        }
    }

    fn config_no_path() -> Config {
        Config {
            project_path: None,
            db_dir: None,
            allow_root: false,
            allow_home: false,
            embedding_model: EmbeddingModel::Small,
            embedding_max_batch_size: 128,
            embedding_cache_dir: None,
            log_file: None,
            log_level: LogLevel::Info,
        }
    }

    #[test]
    fn embedding_models_have_conservative_batch_limits() {
        let cases = [
            (EmbeddingModel::Small, 128),
            (EmbeddingModel::Base, 64),
            (EmbeddingModel::Large, 32),
            (EmbeddingModel::M3, 8),
        ];

        for (model, expected) in cases {
            assert_eq!(model.max_batch_size(), expected);
        }
    }

    #[test]
    fn db_path_has_64_hex_chars_and_db_extension() {
        let path = config("/project").db_path_for("/project");
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with(".db"), "expected .db suffix, got '{name}'");
        let stem = &name[..name.len() - 3];
        assert_eq!(stem.len(), 64, "expected 64 hex chars, got '{stem}'");
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn db_path_is_deterministic() {
        let a = config("/my/project").db_path_for("/my/project");
        let b = config("/my/project").db_path_for("/my/project");
        assert_eq!(a, b);
    }

    #[test]
    fn db_path_differs_per_project() {
        let a = config("/project/a").db_path_for("/project/a");
        let b = config("/project/b").db_path_for("/project/b");
        assert_ne!(a.file_name(), b.file_name());
    }

    #[test]
    fn resolve_project_path_explicit_wins() {
        let c = config("/my/project");
        assert_eq!(
            c.resolve_project_path(&["file:///other".to_string()])
                .unwrap(),
            "/my/project"
        );
    }

    #[test]
    fn resolve_project_path_makes_relative_identity_absolute() {
        let resolved = config("relative-project")
            .resolve_project_path(&[])
            .unwrap();
        assert_eq!(
            Path::new(&resolved),
            std::env::current_dir().unwrap().join("relative-project")
        );
    }

    #[test]
    fn resolve_project_path_uses_first_root_uri() {
        let c = config_no_path();
        let uris = vec!["file:///first".to_string(), "file:///second".to_string()];
        assert_eq!(c.resolve_project_path(&uris).unwrap(), "/first");
    }

    #[test]
    fn resolve_project_path_treats_localhost_as_local_file_uri() {
        let c = config_no_path();
        let direct = c
            .resolve_project_path(&["file:///tmp/my%20project".to_string()])
            .unwrap();
        let localhost = c
            .resolve_project_path(&["file://localhost/tmp/my%20project".to_string()])
            .unwrap();
        assert_eq!(localhost, direct);
        assert_eq!(c.db_path_for(&localhost), c.db_path_for(&direct));
    }

    #[test]
    fn resolve_project_path_decodes_file_uri_percent_escapes() {
        let c = config_no_path();
        let uris = vec!["file:///tmp/my%20project".to_string()];
        assert_eq!(c.resolve_project_path(&uris).unwrap(), "/tmp/my project");
    }

    #[test]
    fn resolve_skips_unusable_root_uris() {
        let uris = [
            "",
            "https://example.com/project",
            "relative-project",
            "file://",
            "file://remote.example.com/project",
            "file://localhost",
            "file:///bad%",
            "file:///bad%GG",
            "file:///bad%FF",
            "file:///bad%00",
            "file:///project?query",
            "file:///project#fragment",
            "file:///valid",
        ]
        .map(str::to_owned);
        assert_eq!(
            config_no_path().resolve_project_path(&uris).unwrap(),
            "/valid"
        );
    }

    #[test]
    fn resolve_decodes_utf8_local_root_uri() {
        assert_eq!(
            config_no_path()
                .resolve_project_path(&[
                    "file://LOCALHOST/first%20project/caf%C3%A9%20%23%25".into()
                ])
                .unwrap(),
            "/first project/café #%"
        );
    }

    #[test]
    fn resolve_falls_back_to_cwd_for_non_file_roots() {
        assert_eq!(
            config_no_path()
                .resolve_project_path(&["https://example.com/project".into()])
                .unwrap(),
            std::env::current_dir().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn resolve_project_path_skips_empty_uri() {
        let c = config_no_path();
        let uris = vec!["file://".to_string(), "file:///valid".to_string()];
        assert_eq!(c.resolve_project_path(&uris).unwrap(), "/valid");
    }

    #[test]
    fn resolve_project_path_rejects_root_path() {
        assert_eq!(
            config("/")
                .resolve_project_path(&[])
                .unwrap_err()
                .to_string(),
            "project path '/' is the filesystem root; use --allow-root to opt in, or set a real project directory via --project-path or MEMLITE_PROJECT_PATH"
        );
    }

    #[test]
    fn resolve_project_path_rejects_root_parent_alias() {
        assert_eq!(
            config("/..")
                .resolve_project_path(&[])
                .unwrap_err()
                .to_string(),
            config("/")
                .resolve_project_path(&[])
                .unwrap_err()
                .to_string()
        );
    }

    #[test]
    fn resolve_project_path_allow_root_parent_alias_opts_in() {
        let mut c = config("/..");
        c.allow_root = true;
        assert_eq!(c.resolve_project_path(&[]).unwrap(), "/");
    }

    #[test]
    fn resolve_project_path_rejects_root_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("root");
        std::os::unix::fs::symlink("/", &link).unwrap();
        assert!(
            config(link.to_str().unwrap())
                .resolve_project_path(&[])
                .is_err()
        );
    }

    #[test]
    fn resolve_project_path_allow_root_opts_in() {
        let mut c = config("/");
        c.allow_root = true;
        assert_eq!(c.resolve_project_path(&[]).unwrap(), "/");
    }

    #[test]
    fn resolve_project_path_rejects_root_uri() {
        // First non-empty URI resolves to "/" and is rejected; no fallthrough to later entries
        let c = config_no_path();
        let uris = vec!["file:///".to_string(), "file:///real/project".to_string()];
        assert_eq!(
            c.resolve_project_path(&uris).unwrap_err().to_string(),
            "project path '/' is the filesystem root; use --allow-root to opt in, or set a real project directory via --project-path or MEMLITE_PROJECT_PATH"
        );
    }

    #[test]
    fn resolve_project_path_uses_cwd_when_no_roots() {
        // CWD is always available in a test process (and is never "/")
        assert!(config_no_path().resolve_project_path(&[]).is_ok());
    }

    #[test]
    fn resolve_project_path_rejects_home_dir() {
        let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned)) else {
            return; // skip when home dir is unavailable
        };
        assert_eq!(
            config(&home)
                .resolve_project_path(&[])
                .unwrap_err()
                .to_string(),
            "project path is the home directory; use --allow-home to opt in, or set a specific project directory via --project-path or MEMLITE_PROJECT_PATH"
        );
    }

    #[test]
    fn resolve_project_path_rejects_home_dir_with_trailing_slash() {
        let Some(mut home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned)) else {
            return;
        };
        if !home.ends_with('/') {
            home.push('/');
        }
        assert_eq!(
            config(&home)
                .resolve_project_path(&[])
                .unwrap_err()
                .to_string(),
            "project path is the home directory; use --allow-home to opt in, or set a specific project directory via --project-path or MEMLITE_PROJECT_PATH"
        );
    }

    #[test]
    fn resolve_project_path_allow_home_opts_in() {
        let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned)) else {
            return;
        };
        let mut c = config(&home);
        c.allow_home = true;
        assert_eq!(c.resolve_project_path(&[]).unwrap(), home);
    }

    #[test]
    fn embedding_model_from_maps_all_variants() {
        assert!(matches!(
            fastembed::EmbeddingModel::from(EmbeddingModel::Small),
            fastembed::EmbeddingModel::BGESmallENV15
        ));
        assert!(matches!(
            fastembed::EmbeddingModel::from(EmbeddingModel::Base),
            fastembed::EmbeddingModel::BGEBaseENV15
        ));
        assert!(matches!(
            fastembed::EmbeddingModel::from(EmbeddingModel::Large),
            fastembed::EmbeddingModel::BGELargeENV15
        ));
        assert!(matches!(
            fastembed::EmbeddingModel::from(EmbeddingModel::M3),
            fastembed::EmbeddingModel::BGEM3
        ));
    }
}
