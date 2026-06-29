/// Error returned by [`crate::config::Config::resolve_project_path`] when a valid path cannot be produced.
#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// No source (explicit arg, root URIs, or CWD) yielded a usable path.
    NotFound,
    /// The resolved path is the filesystem root; use `--allow-root` to opt in.
    RootPath,
    /// The resolved path is the home directory; use `--allow-home` to opt in.
    HomePath,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(
                f,
                "cannot determine project path: \
                 set --project-path or MEMLITE_PROJECT_PATH"
            ),
            Self::RootPath => write!(
                f,
                "project path '/' is the filesystem root; \
                 use --allow-root to opt in, or set a real project directory \
                 via --project-path or MEMLITE_PROJECT_PATH"
            ),
            Self::HomePath => write!(
                f,
                "project path is the home directory; \
                 use --allow-home to opt in, or set a specific project directory \
                 via --project-path or MEMLITE_PROJECT_PATH"
            ),
        }
    }
}
