# Configuration and CLI Reference

The defaults are sane. Run `memlite-mcp serve` for the MCP server, or
`memlite-mcp tool <command>` for direct memory management. This document
covers every command, subcommand, and option; for the shape of a memory
record, see [memory.md](memory.md), and for per-tool semantics, see
[tools.md](tools.md).

## Global options

All shared options are available as global CLI flags and env vars (Clap
resolves both; flags take precedence). Global flags work before or after
nested commands, under both `serve` and `tool`. Run `memlite-mcp --help` for
Clap's own generated usage listing.

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--project-path` | `MEMLITE_PROJECT_PATH` | first MCP workspace root under `serve`, or CWD under `tool` | Project identity; resolved to an absolute path before selecting the shared DB |
| `--db-dir` | `MEMLITE_DB_DIR` | OS data dir / `memlite-mcp` | Directory for per-project DB files (`~/.local/share/…` on Linux, `~/Library/Application Support/…` on macOS) |
| `--allow-root` | `MEMLITE_ALLOW_ROOT` | `false` | Allow `/` as the project path (off by default; almost always wrong) |
| `--allow-home` | `MEMLITE_ALLOW_HOME` | `false` | Allow the home directory as the project path (off by default; spans unrelated projects) |
| `--embedding-model` | `MEMLITE_EMBEDDING_MODEL` | `bge-small-en-v1.5` | See [embedding models](#embedding-models) below |
| `--embedding-max-batch-size` | `MEMLITE_EMBEDDING_MAX_BATCH_SIZE` | `128` | Requested inference batch cap; effective size also respects model tier cap |
| `--embedding-cache-dir` | `MEMLITE_EMBEDDING_CACHE_DIR` | OS cache dir / `memlite-mcp` | Override where model files are cached |
| `--log-file` | `MEMLITE_LOG_FILE` | — | Write logs to this file instead of stderr |
| `--log-level` | `MEMLITE_LOG_LEVEL` | `info` | Minimum log level when `RUST_LOG` is not set (`trace`/`debug`/`info`/`warn`/`error`) |
| — | `RUST_LOG` | — | Standard `tracing` filter string; overrides `--log-level` when set |

`--allow-root` and `--allow-home` take an explicit `true`/`false` value
(e.g. `--allow-root true`) rather than acting as bare presence flags, so a
truthy env var can still be overridden from the command line.

## Project identity

MemLite hashes the resolved project path to select its database. Explicit
relative paths become absolute before hashing, matching GraphLite and avoiding
separate databases for equivalent relative and absolute inputs.

MCP workspace roots remain a compatibility fallback. Multi-root hosts may start
each server process with a different current working directory, so their
automatic fallback is not a stable workspace identity. Set one absolute
`MEMLITE_PROJECT_PATH` value in the MCP host configuration and use the same
value for direct commands that should share its memories. For example, add this
to the existing Zed context server command:

```json
"env": {
  "MEMLITE_PROJECT_PATH": "/Users/me/Development/my-workspace"
}
```

The explicit value selects the database without requesting deprecated MCP
roots. It may identify the common workspace rather than one repository. Changing
it selects a different database and does not migrate existing memories.

For file references inside a multi-root database, prefix each path with its
repository root directory name, such as `memlite-mcp/src/config.rs`. See
[references](memory.md#references) for the full convention.

## Shared worker election

Every `serve` process starts as a peer. First process for a canonical database
directory takes `worker.lock` and hosts memory operations for all connected MCP
sessions. Later processes retain their editor-facing MCP session and proxy tool
operations over a private mode-0600 Unix socket in `/tmp`. There is no separate
daemon command or detached process.

Worker keeps one lazy `MemoryStore` per active project. Read-only sessions do not
open or create absent project databases, preserving MemLite's empty-project
behavior. Embedding services also stay lazy and are shared by model across
projects. Different models can coexist in one worker. Sessions sharing one
project must select same model, batch cap, and cache directory; conflicting
settings fail during initialization instead of creating duplicate resources. Existing database
metadata still rejects a different model for an already populated project.

When worker process exits, operating system releases its lock. Connected proxies
re-elect immediately; one promotes and others reconnect. Client MCP sessions
remain open. In-flight requests are not replayed because a write may already have
committed. One overlapping request can therefore fail and should be retried.

Project and model registry entries are weak. Store and model resources unload
when no MCP session still uses them.

## Embedding models

`--embedding-max-batch-size` sets a user cap. The effective inference batch size
is the smaller of that value and the selected model's conservative tier cap.
This limits batching passed to FastEmbed; it is not a claimed hard process
memory limit.

| Model | Dimensions | Download size | Tier cap |
|---|---:|---:|---:|
| `bge-small-en-v1.5` (default) | 384 | ~67 MB | 128 |
| `bge-base-en-v1.5` | 768 | ~210 MB | 64 |
| `bge-large-en-v1.5` | 1024 | ~620 MB | 32 |
| `bge-m3` | 1024 | ~2.3 GB | 8 |

The v1.5 models are English-first; larger variants trade download size and
inference time for retrieval quality. `bge-m3` supports 100+ languages and an
8192-token context, at a much larger download and runtime cost.

The model is downloaded on the first MCP `memory_store` or `memory_search`
call, or the first direct `tool store`, `tool search`, or content-changing
`tool update` command. It does not load for read-only commands or at server
start, so the first embedding operation can be slower. It is cached in the OS
cache directory (`~/Library/Caches/memlite-mcp` on macOS,
`~/.cache/memlite-mcp` on Linux) and subsequent calls are fully offline.

Note that the model is recorded in each project DB on first use, and
opening the DB with a different model is an error (it would mix vectors of
different dimensions). Switching models for an existing project means
starting a fresh DB.

## Commands

```
memlite-mcp [GLOBAL OPTIONS] <COMMAND>
```

| Command | Description |
|---|---|
| [`serve`](#serve) | Runs the MCP server over stdio |
| [`tool`](#tool) | Runs one memory tool directly against the project database |

A subcommand is required; `memlite-mcp` alone prints usage and exits.

### `serve`

Runs the MCP server over stdio. Takes no options beyond the
[global options](#global-options).

```sh
memlite-mcp serve
```

MCP host configurations should invoke this subcommand directly. The server
exposes 12 `memory_*` tools, documented in [tools.md](tools.md).

### `tool`

Runs one memory operation directly against the project database and writes
the result to stdout.

```
memlite-mcp tool [--format <text|json>] <TOOL COMMAND>
```

| Option | Default | Description |
|---|---|---|
| `--format <text\|json>` | `text` | Output format for the command below; `text` matches MCP block text and TOON shapes, `json` emits structured JSON |

Conventions shared by every subcommand below:

- Required scalar inputs (queries, identifiers) are positional; everything
  else is a kebab-case flag.
- Repeat `--tag`, `--relation ID:KIND`, and `--ref` to pass multiple values.
- `tool update` provides `--clear-tags`, `--clear-attrs`, `--clear-relations`,
  and `--clear-refs`, because those fields are full-set replacements, not
  merges.
- Boolean flags that take an explicit value (`--include-content`,
  `--reversible`) require `true` or `false`, not bare presence.
- `tool store` stores one memory per invocation; MCP `memory_store` remains
  the batch interface for 1 to 100 atomic inserts.
- Run `memlite-mcp tool <command> --help` for Clap's own generated usage
  line.

Each subcommand below wraps the identically named MCP tool; see
[tools.md](tools.md) for full parameter semantics, defaults, and behavior.

#### `tool store`

Stores one memory. Wraps [`memory_store`](tools.md#memory_store).

```
memlite-mcp tool store <CONTENT> --memory-type <TYPE> --summary <SUMMARY> [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `content` | positional, required | Full text to embed and store |
| `--memory-type` | flag, required | Structured memory category |
| `--summary` | flag, required | One-sentence description for list views; blank values are rejected |
| `--tag` | flag, repeatable | Grouping label |
| `--context` | flag | Background rationale |
| `--confidence` | flag | Initial confidence score in the range 0 to 1 |
| `--decay-rate` | flag | Per-day confidence decay rate in the range 0 to 1 |
| `--status`, `--severity`, `--priority`, `--outcome`, `--phase` | flag | Typed attrs; see [memory.md](memory.md#typed-attrs) |
| `--reversible <BOOL>` | flag | Typed attrs; see [memory.md](memory.md#typed-attrs) |
| `--relation <ID:KIND>` | flag, repeatable | Outgoing relation |
| `--ref` | flag, repeatable | External reference |

#### `tool search`

Searches memories by semantic similarity. Wraps
[`memory_search`](tools.md#memory_search).

```
memlite-mcp tool search <QUERY> [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `query` | positional, required | Natural-language query |
| `--limit` | flag | Maximum number of results |
| `--include-content <BOOL>` | flag | Includes full context and content |
| `--tag` | flag, repeatable | Required tag (AND filtering) |
| `--memory-type` | flag | Required memory category |
| `--status`, `--severity`, `--priority`, `--outcome`, `--phase` | flag | Typed attrs filters |
| `--refs-contain` | flag | Required substring in an external reference |

#### `tool list`

Lists memories newest first. Wraps [`memory_list`](tools.md#memory_list).

```
memlite-mcp tool list [IDS...] [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `ids` | positional, repeatable | Exact memory identifiers; missing identifiers are omitted |
| `--limit` | flag | Maximum number of results per page |
| `--offset` | flag | Zero-based pagination offset |
| `--include-content <BOOL>` | flag | Includes full context and content |
| `--tag` | flag, repeatable | Required tag (AND filtering) |
| `--memory-type` | flag | Required memory category |
| `--status`, `--severity`, `--priority`, `--outcome`, `--phase` | flag | Typed attrs filters |
| `--refs-contain` | flag | Required substring in an external reference |

#### `tool tags`

Lists tags and occurrence counts. Wraps [`memory_tags`](tools.md#memory_tags).
Takes no arguments beyond `--format`.

```sh
memlite-mcp tool tags
```

#### `tool delete`

Deletes one memory. Wraps [`memory_delete`](tools.md#memory_delete).

```
memlite-mcp tool delete <ID>
```

| Argument | Kind | Description |
|---|---|---|
| `id` | positional, required | Numeric memory identifier |

#### `tool update`

Updates one memory. Wraps [`memory_update`](tools.md#memory_update).

```
memlite-mcp tool update <ID> [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `id` | positional, required | Numeric memory identifier |
| `--content` | flag | Replacement content; triggers re-embedding |
| `--summary` | flag | Replacement summary; blank values are rejected |
| `--tag` | flag, repeatable | Replacement tag |
| `--clear-tags` | flag (presence) | Clears all tags |
| `--context` | flag | Replacement context; pass an empty string to clear |
| `--confidence` | flag | Replacement confidence score in the range 0 to 1 |
| `--decay-rate` | flag | Replacement per-day decay rate in the range 0 to 1 |
| `--memory-type` | flag | Replacement memory category |
| `--status`, `--severity`, `--priority`, `--outcome`, `--phase` | flag | Replacement typed attrs |
| `--reversible <BOOL>` | flag | Replacement typed attrs |
| `--clear-attrs` | flag (presence) | Clears all typed attributes |
| `--relation <ID:KIND>` | flag, repeatable | Replacement relation |
| `--clear-relations` | flag (presence) | Clears all outgoing relations |
| `--ref` | flag, repeatable | Replacement external reference |
| `--clear-refs` | flag (presence) | Clears all external references |

#### `tool revalidate`

Resets confidence decay clocks. Wraps
[`memory_revalidate`](tools.md#memory_revalidate).

```
memlite-mcp tool revalidate <IDS...>
```

| Argument | Kind | Description |
|---|---|---|
| `ids` | positional, repeatable, required (1 to 100) | Numeric memory identifiers |

#### `tool neighbors`

Lists immediate incoming and outgoing neighbors. Wraps
[`memory_neighbors`](tools.md#memory_neighbors).

```
memlite-mcp tool neighbors <ID> [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `id` | positional, required | Numeric memory identifier |
| `--kind` | flag | Exact relation kind filter |

#### `tool traverse`

Traverses the relation graph breadth first. Wraps
[`memory_traverse`](tools.md#memory_traverse).

```
memlite-mcp tool traverse <START_ID> [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `start_id` | positional, required | Numeric starting memory identifier |
| `--depth` | flag | Traversal depth, 1 through 5 |
| `--kind` | flag | Exact relation kind filter |
| `--direction <out\|in\|both>` | flag | Edge direction to follow |

#### `tool path`

Finds the shortest directed path between memories. Wraps
[`memory_path`](tools.md#memory_path).

```
memlite-mcp tool path <FROM_ID> <TO_ID> [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `from_id` | positional, required | Numeric starting memory identifier |
| `to_id` | positional, required | Numeric destination memory identifier |
| `--max-hops` | flag | Maximum hops, 1 through 10 |

#### `tool hubs`

Lists the most connected memories. Wraps [`memory_hubs`](tools.md#memory_hubs).

```
memlite-mcp tool hubs [OPTIONS]
```

| Argument | Kind | Description |
|---|---|---|
| `--top-n` | flag | Maximum hub count, 1 through 50 |

#### `tool stats`

Shows graph-level statistics. Wraps [`memory_stats`](tools.md#memory_stats).
Takes no arguments beyond `--format`.

```sh
memlite-mcp tool stats
```
