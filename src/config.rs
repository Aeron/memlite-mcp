use std::path::PathBuf;

/// Runtime configuration populated from environment variables at startup.
pub struct Config {
    /// Identifies the project; used to derive the per-project DB filename.
    pub project_path: String,
    /// Stored on each memory row for per-agent filtering.
    pub agent_id: String,
    /// Directory that holds all per-project DB files.
    pub db_dir: PathBuf,
    /// When true, opens an in-memory DB instead of a file.
    pub in_memory: bool,
}

impl Config {
    /// Reads configuration from well-known environment variables with defaults.
    ///
    /// | Variable              | Default                                    |
    /// |-----------------------|--------------------------------------------|
    /// | MEMLITE_PROJECT_PATH  | "" (no filtering; all memories visible)    |
    /// | MEMLITE_AGENT_ID      | "" (no agent filtering)                    |
    /// | MEMLITE_DB_DIR        | $XDG_DATA_HOME/memlite-mcp                 |
    /// | MEMLITE_IN_MEMORY     | "false"                                    |
    pub fn from_env() -> Self {
        let project_path = std::env::var("MEMLITE_PROJECT_PATH").unwrap_or_default();
        let agent_id = std::env::var("MEMLITE_AGENT_ID").unwrap_or_default();
        let in_memory = std::env::var("MEMLITE_IN_MEMORY")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let db_dir = std::env::var("MEMLITE_DB_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("memlite-mcp")
            });
        Config { project_path, agent_id, db_dir, in_memory }
    }

    /// Returns the DB file path for the current project, or None for in-memory mode.
    ///
    /// The filename is a hex representation of a stable hash of `project_path`
    /// so that the same project always maps to the same file without an index.
    pub fn db_path(&self) -> Option<PathBuf> {
        if self.in_memory {
            return None;
        }
        Some(self.db_dir.join(format!("{:016x}.db", project_hash(&self.project_path))))
    }

    /// Returns the optional agent_id filter: Some when non-empty, None otherwise.
    pub fn agent_filter(&self) -> Option<&str> {
        if self.agent_id.is_empty() { None } else { Some(&self.agent_id) }
    }
}

fn project_hash(path: &str) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}
