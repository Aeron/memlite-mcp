# Configuration

The defaults are sane; running `memlite-mcp` with no arguments works for
most setups. All options are available as CLI flags and env vars (Clap
resolves both; flags take precedence). Run `memlite-mcp --help` for the full
usage listing.

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--project-path` | `MEMLITE_PROJECT_PATH` | first MCP workspace root, or CWD | Project identifier; agents sharing the same path share a DB |
| `--db-dir` | `MEMLITE_DB_DIR` | OS data dir / `memlite-mcp` | Directory for per-project DB files (`~/.local/share/…` on Linux, `~/Library/Application Support/…` on macOS) |
| `--allow-root` | `MEMLITE_ALLOW_ROOT` | `false` | Allow `/` as the project path (off by default; almost always wrong) |
| `--allow-home` | `MEMLITE_ALLOW_HOME` | `false` | Allow the home directory as the project path (off by default; spans unrelated projects) |
| `--embedding-model` | `MEMLITE_EMBEDDING_MODEL` | `bge-small-en-v1.5` | See models table below |
| `--embedding-cache-dir` | `MEMLITE_EMBEDDING_CACHE_DIR` | OS cache dir / `memlite-mcp` | Override where model files are cached |
| `--log-file` | `MEMLITE_LOG_FILE` | — | Write logs to this file instead of stderr |
| `--log-level` | `MEMLITE_LOG_LEVEL` | `info` | Minimum log level when `RUST_LOG` is not set (`trace`/`debug`/`info`/`warn`/`error`) |
| — | `RUST_LOG` | — | Standard `tracing` filter string; overrides `--log-level` when set |

## Embedding models

| Model | Dimensions | Download size |
|---|---|---|
| `bge-small-en-v1.5` (default) | 384 | ~67 MB |
| `bge-base-en-v1.5` | 768 | ~210 MB |
| `bge-large-en-v1.5` | 1024 | ~620 MB |

All three are English-first models; larger variants trade download size and
inference time for retrieval quality.

The model is downloaded on the first `memory_store` or `memory_search` call,
not at server start, so the very first tool call can be slower. It is cached
in the OS cache directory (`~/Library/Caches/memlite-mcp` on macOS,
`~/.cache/memlite-mcp` on Linux) and subsequent calls are fully offline.

Note that the model is recorded in each project DB on first use, and
opening the DB with a different model is an error (it would mix vectors of
different dimensions). Switching models for an existing project means
starting a fresh DB.
