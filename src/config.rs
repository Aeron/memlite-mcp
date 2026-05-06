use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use clap::{Parser, ValueEnum};

/// Local-only MCP server for persistent agent memory.
#[derive(Parser)]
#[command(name = "memlite-mcp", version)]
pub struct Config {
    /// Project identifier shared across agents. All agents with the same path
    /// read and write the same DB file. Defaults to the first workspace root
    /// reported by the MCP client, or $HOME if none is available.
    #[arg(long = "project-path", env = "MEMLITE_PROJECT_PATH")]
    project_path: Option<String>,

    /// Agent name stored on each memory row. Empty means no filtering:
    /// all memories in the project are visible.
    #[arg(long = "agent", env = "MEMLITE_AGENT", default_value = "")]
    pub agent: String,

    /// Directory for per-project DB files. Defaults to the XDG data home.
    #[arg(long = "db-dir", env = "MEMLITE_DB_DIR")]
    db_dir: Option<PathBuf>,

    /// Use an ephemeral in-memory DB (no persistence across restarts).
    #[arg(long = "in-memory", env = "MEMLITE_IN_MEMORY", default_value_t = false, value_parser = parse_bool)]
    pub in_memory: bool,

    /// Embedding model to use for new memories.
    #[arg(
        long = "embedding-model",
        env = "MEMLITE_EMBEDDING_MODEL",
        default_value_t = EmbeddingModel::Small
    )]
    pub embedding_model: EmbeddingModel,

    /// Directory for cached embedding model files. Defaults to `<db-dir>/fastembed_cache`.
    #[arg(long = "embedding-cache-dir", env = "MEMLITE_EMBEDDING_CACHE_DIR")]
    embedding_cache_dir: Option<PathBuf>,

    /// Write logs to this file instead of stderr. Useful when the MCP host does not
    /// surface the server's stderr (e.g. Zed has no MCP log panel).
    #[arg(long = "log-file", env = "MEMLITE_LOG_FILE")]
    pub log_file: Option<PathBuf>,
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
            None => self.db_dir().join("fastembed_cache"),
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

    /// Returns the DB file path for the given project path, or None in in-memory mode.
    pub fn db_path_for(&self, project_path: &str) -> Option<PathBuf> {
        if self.in_memory {
            return None;
        }
        Some(
            self.db_dir()
                .join(format!("{}.db", Self::project_path_hash(project_path))),
        )
    }

    /// Resolves the effective project path from the explicit override or MCP roots.
    ///
    /// Priority: explicit arg > first valid `file://` root URI > CWD.
    /// Returns None when none of the sources yield a usable path.
    pub fn resolve_project_path(explicit: Option<&str>, root_uris: &[String]) -> Option<String> {
        if let Some(p) = explicit {
            return Some(p.to_owned());
        }
        for uri in root_uris {
            let path = uri.strip_prefix("file://").unwrap_or(uri.as_str());
            if !path.is_empty() {
                return Some(path.to_owned());
            }
        }
        std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
    }

    /// Returns Some(agent) when non-empty, for use as an optional query filter.
    pub fn agent_filter(&self) -> Option<&str> {
        if self.agent.is_empty() {
            None
        } else {
            Some(&self.agent)
        }
    }

    /// Returns a SHA-256 hex digest of the given project path string.
    pub fn project_path_hash(project_path: &str) -> String {
        let hash = Sha256::digest(project_path.as_bytes());
        format!("{hash:x}")
    }
}

/// Parses a bool from a CLI string.
fn parse_bool(s: &str) -> Result<bool, String> {
    match s {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(format!("expected true/false/1/0, got '{s}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(project_path: &str, in_memory: bool) -> Config {
        Config {
            project_path: Some(project_path.into()),
            agent: String::new(),
            db_dir: None,
            in_memory,
            embedding_model: EmbeddingModel::Small,
            embedding_cache_dir: None,
            log_file: None,
        }
    }

    #[test]
    fn db_path_none_when_in_memory() {
        assert!(config("/project", true).db_path_for("/project").is_none());
    }

    #[test]
    fn db_path_has_64_hex_chars_and_db_extension() {
        let path = config("/project", false).db_path_for("/project").unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with(".db"), "expected .db suffix, got '{name}'");
        let stem = &name[..name.len() - 3];
        assert_eq!(stem.len(), 64, "expected 64 hex chars, got '{stem}'");
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn db_path_is_deterministic() {
        let a = config("/my/project", false)
            .db_path_for("/my/project")
            .unwrap();
        let b = config("/my/project", false)
            .db_path_for("/my/project")
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn db_path_differs_per_project() {
        let a = config("/project/a", false)
            .db_path_for("/project/a")
            .unwrap();
        let b = config("/project/b", false)
            .db_path_for("/project/b")
            .unwrap();
        assert_ne!(a.file_name(), b.file_name());
    }

    #[test]
    fn resolve_project_path_explicit_wins() {
        let path =
            Config::resolve_project_path(Some("/my/project"), &["file:///other".to_string()]);
        assert_eq!(path, Some("/my/project".to_owned()));
    }

    #[test]
    fn resolve_project_path_uses_first_root_uri() {
        let uris = vec!["file:///first".to_string(), "file:///second".to_string()];
        assert_eq!(
            Config::resolve_project_path(None, &uris),
            Some("/first".to_owned())
        );
    }

    #[test]
    fn resolve_project_path_skips_empty_after_strip() {
        let uris = vec!["file://".to_string(), "file:///valid".to_string()];
        assert_eq!(
            Config::resolve_project_path(None, &uris),
            Some("/valid".to_owned())
        );
    }

    #[test]
    fn resolve_project_path_uses_cwd_when_no_roots() {
        // CWD is always available in a test process.
        let path = Config::resolve_project_path(None, &[]);
        assert!(path.is_some());
    }

    #[test]
    fn agent_filter_empty_is_none() {
        assert!(config("/p", false).agent_filter().is_none());
    }

    #[test]
    fn agent_filter_non_empty_is_some() {
        let mut c = config("/p", false);
        c.agent = "claude".into();
        assert_eq!(c.agent_filter(), Some("claude"));
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
