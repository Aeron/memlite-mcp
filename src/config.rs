use std::{
    borrow::Cow,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
};

use clap::{Parser, ValueEnum};

/// Local-only MCP server for persistent agent memory.
#[derive(Parser)]
#[command(name = "memlite-mcp", version)]
pub struct Config {
    /// Project identifier shared across agents. All agents with the same path
    /// read and write the same DB file. Defaults to the current working directory.
    #[arg(
        long = "project-path",
        env = "MEMLITE_PROJECT_PATH",
        default_value_t = cwd_string()
    )]
    pub project_path: String,

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

    /// Returns the DB file path for the current project, or None in in-memory mode.
    pub fn db_path(&self) -> Option<PathBuf> {
        if self.in_memory {
            return None;
        }
        Some(
            self.db_dir()
                .join(format!("{:016x}.db", self.project_hash())),
        )
    }

    /// Returns Some(agent) when non-empty, for use as an optional query filter.
    pub fn agent_filter(&self) -> Option<&str> {
        if self.agent.is_empty() {
            None
        } else {
            Some(&self.agent)
        }
    }

    fn project_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.project_path.hash(&mut h);
        h.finish()
    }
}

/// Returns the current working directory as a string.
fn cwd_string() -> String {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .to_string_lossy()
        .into_owned()
}

/// Parses a boolean value from a string, returning an error if the string is not a valid boolean.
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
            project_path: project_path.into(),
            agent: String::new(),
            db_dir: None,
            in_memory,
            embedding_model: EmbeddingModel::Small,
        }
    }

    #[test]
    fn db_path_none_when_in_memory() {
        assert!(config("/project", true).db_path().is_none());
    }

    #[test]
    fn db_path_has_16_hex_chars_and_db_extension() {
        let path = config("/project", false).db_path().unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.ends_with(".db"), "expected .db suffix, got '{name}'");
        let stem = &name[..name.len() - 3];
        assert_eq!(stem.len(), 16, "expected 16 hex chars, got '{stem}'");
        assert!(stem.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn db_path_is_deterministic() {
        let a = config("/my/project", false).db_path().unwrap();
        let b = config("/my/project", false).db_path().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn db_path_differs_per_project() {
        let a = config("/project/a", false).db_path().unwrap();
        let b = config("/project/b", false).db_path().unwrap();
        assert_ne!(a.file_name(), b.file_name());
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
