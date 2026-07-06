use std::{
    borrow::Cow,
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use strum::Display;
use tracing_subscriber::{EnvFilter, fmt};

use crate::errors::ResolveError;

/// Local-only MCP server for persistent agent memory.
#[derive(Parser)]
#[command(name = "memlite-mcp", version)]
pub struct Config {
    /// Project identifier shared across agents. All agents with the same path
    /// read and write the same DB file. Defaults to the first workspace root
    /// reported by the MCP client, or CWD if none is available.
    #[arg(long = "project-path", env = "MEMLITE_PROJECT_PATH")]
    project_path: Option<String>,

    /// Directory for per-project DB files. Defaults to the XDG data home.
    #[arg(long = "db-dir", env = "MEMLITE_DB_DIR")]
    db_dir: Option<PathBuf>,

    /// Allows the filesystem root ("/") as the project path.
    /// Off by default; using the whole filesystem as a project is almost certainly wrong.
    #[arg(long = "allow-root", env = "MEMLITE_ALLOW_ROOT", default_value_t = false, value_parser = clap::builder::BoolishValueParser::new())]
    allow_root: bool,

    /// Allows the home directory as the project path.
    /// Off by default; the home directory spans many unrelated projects.
    #[arg(long = "allow-home", env = "MEMLITE_ALLOW_HOME", default_value_t = false, value_parser = clap::builder::BoolishValueParser::new())]
    allow_home: bool,

    /// Embedding model to use for new memories.
    #[arg(
        long = "embedding-model",
        env = "MEMLITE_EMBEDDING_MODEL",
        default_value_t = EmbeddingModel::Small
    )]
    pub embedding_model: EmbeddingModel,

    /// Directory for cached embedding model files. Defaults to `<cache-dir>/memlite-mcp`.
    #[arg(long = "embedding-cache-dir", env = "MEMLITE_EMBEDDING_CACHE_DIR")]
    embedding_cache_dir: Option<PathBuf>,

    /// Write logs to this file instead of stderr. Useful when the MCP host does not
    /// surface the server's stderr (e.g. Zed has no MCP log panel).
    #[arg(long = "log-file", env = "MEMLITE_LOG_FILE")]
    pub log_file: Option<PathBuf>,

    /// Minimum log level when `RUST_LOG` is not set. Applies to both file and stderr output.
    #[arg(long = "log-level", env = "MEMLITE_LOG_LEVEL", default_value_t = LogLevel::Info)]
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
#[derive(Debug, Clone, Copy, ValueEnum)]
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
        }
    }
}

impl Config {
    /// Returns the resolved embedding model cache directory.
    pub fn embedding_cache_dir(&self) -> PathBuf {
        match &self.embedding_cache_dir {
            Some(p) => p.clone(),
            None => dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("memlite-mcp"),
        }
    }

    /// Returns the resolved DB directory.
    ///
    /// Borrows when `--db-dir` was provided explicitly, allocating only for
    /// the XDG default fallback.
    pub fn db_dir(&self) -> Cow<'_, Path> {
        match &self.db_dir {
            Some(p) => Cow::Borrowed(p.as_path()),
            None => Cow::Owned(
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("memlite-mcp"),
            ),
        }
    }

    /// Returns the raw CLI/env project path, if provided explicitly.
    pub fn explicit_project_path(&self) -> Option<&str> {
        self.project_path.as_deref()
    }

    /// Returns the DB file path for the given project path.
    pub fn db_path_for(&self, project_path: &str) -> PathBuf {
        self.db_dir()
            .join(format!("{}.db", Self::project_path_hash(project_path)))
    }

    /// Resolves the effective project path from the explicit override or MCP roots.
    ///
    /// Priority: explicit arg > first non-empty `file://` root URI > CWD.
    /// Validates the candidate once: rejects the filesystem root unless `--allow-root` is set,
    /// and rejects the home directory unless `--allow-home` is set.
    pub fn resolve_project_path(&self, root_uris: &[String]) -> Result<String, ResolveError> {
        let path = if let Some(p) = self.project_path.as_deref() {
            p.to_owned()
        } else {
            root_uris
                .iter()
                .filter_map(|uri| {
                    let p = uri
                        .strip_prefix("file://")
                        .map(decode_percent)
                        .unwrap_or_else(|| uri.to_owned());
                    (!p.is_empty()).then_some(p)
                })
                .next()
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .and_then(|p| p.to_str().map(str::to_owned))
                })
                .ok_or(ResolveError::NotFound)?
        };

        if !self.allow_root && path == "/" {
            return Err(ResolveError::RootPath);
        }
        if !self.allow_home && is_home_path(&path) {
            return Err(ResolveError::HomePath);
        }
        Ok(path)
    }

    /// Initialises the global tracing subscriber based on `log_file` and `log_level`.
    ///
    /// Writes to the file when set, falling back to stderr. Respects `RUST_LOG` when
    /// set; otherwise uses `--log-level` (default: info). Must be called once at
    /// startup before any `tracing` macros are used.
    pub fn init_logging(&self) -> Result<()> {
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

    /// Returns a SHA-256 hex digest of the given project path string.
    pub fn project_path_hash(project_path: &str) -> String {
        let hash = Sha256::digest(project_path.as_bytes());
        format!("{hash:x}")
    }
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

fn decode_percent(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::errors::ResolveError;

    fn config(project_path: &str) -> Config {
        Config {
            project_path: Some(project_path.into()),
            db_dir: None,
            allow_root: false,
            allow_home: false,
            embedding_model: EmbeddingModel::Small,
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
            embedding_cache_dir: None,
            log_file: None,
            log_level: LogLevel::Info,
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
            c.resolve_project_path(&["file:///other".to_string()]),
            Ok("/my/project".to_owned())
        );
    }

    #[test]
    fn resolve_project_path_uses_first_root_uri() {
        let c = config_no_path();
        let uris = vec!["file:///first".to_string(), "file:///second".to_string()];
        assert_eq!(c.resolve_project_path(&uris), Ok("/first".to_owned()));
    }

    #[test]
    fn resolve_project_path_decodes_file_uri_percent_escapes() {
        let c = config_no_path();
        let uris = vec!["file:///tmp/my%20project".to_string()];
        assert_eq!(
            c.resolve_project_path(&uris),
            Ok("/tmp/my project".to_owned())
        );
    }

    #[test]
    fn resolve_project_path_skips_empty_uri() {
        let c = config_no_path();
        let uris = vec!["file://".to_string(), "file:///valid".to_string()];
        assert_eq!(c.resolve_project_path(&uris), Ok("/valid".to_owned()));
    }

    #[test]
    fn resolve_project_path_rejects_root_path() {
        assert_eq!(
            config("/").resolve_project_path(&[]),
            Err(ResolveError::RootPath)
        );
    }

    #[test]
    fn resolve_project_path_allow_root_opts_in() {
        let mut c = config("/");
        c.allow_root = true;
        assert_eq!(c.resolve_project_path(&[]), Ok("/".to_owned()));
    }

    #[test]
    fn resolve_project_path_rejects_root_uri() {
        // First non-empty URI resolves to "/" and is rejected; no fallthrough to later entries
        let c = config_no_path();
        let uris = vec!["file:///".to_string(), "file:///real/project".to_string()];
        assert_eq!(c.resolve_project_path(&uris), Err(ResolveError::RootPath));
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
            config(&home).resolve_project_path(&[]),
            Err(ResolveError::HomePath)
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
            config(&home).resolve_project_path(&[]),
            Err(ResolveError::HomePath)
        );
    }

    #[test]
    fn resolve_project_path_allow_home_opts_in() {
        let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_owned)) else {
            return;
        };
        let mut c = config(&home);
        c.allow_home = true;
        assert_eq!(c.resolve_project_path(&[]), Ok(home));
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
    }
}
