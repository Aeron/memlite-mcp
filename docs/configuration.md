# Configuration and CLI reference

Run `memlite serve` for the MCP server or `memlite tool <command>`
for direct memory management. Both use the same project database.

This reference covers commands, flags, environment variables, and storage.
See [client setup](clients.md) to connect an agent, [memory.md](memory.md)
for memory records, and [tools.md](tools.md) for tool behavior.

## Global options

Shared options accept both CLI flags and environment variables. Flags take
precedence. Global flags work before or after nested commands under both
`serve` and `tool`. Run `memlite --help` for generated usage.

| Flag | Environment variable | Default | Description |
|---|---|---|---|
| `--project-path` | `MEMLITE_PROJECT_PATH` | First usable MCP workspace root under `serve`, then the working directory. The working directory under `tool`. | Project identity, resolved to an absolute path before database selection. |
| `--db-dir` | `MEMLITE_DB_DIR` | OS data directory / `memlite-mcp` | Directory for per-project databases. See [storage locations](#storage-locations). |
| `--allow-root` | `MEMLITE_ALLOW_ROOT` | `false` | Allows `/` as the project path. This shares one database across the filesystem. |
| `--allow-home` | `MEMLITE_ALLOW_HOME` | `false` | Allows the home directory as the project path. This can combine unrelated projects. |
| `--embedding-model` | `MEMLITE_EMBEDDING_MODEL` | `bge-small-en-v1.5` | Selects an [embedding model](#embedding-models). |
| `--embedding-max-batch-size` | `MEMLITE_EMBEDDING_MAX_BATCH_SIZE` | `128` | Caps inference batches. The model tier can impose a lower cap. |
| `--embedding-cache-dir` | `MEMLITE_EMBEDDING_CACHE_DIR` | OS cache directory / `memlite-mcp` | Overrides the model cache location. |
| `--log-file` | `MEMLITE_LOG_FILE` | None | Writes logs to this file instead of stderr. |
| `--log-level` | `MEMLITE_LOG_LEVEL` | `info` | Sets the minimum log level when `RUST_LOG` is absent. Accepts `trace`, `debug`, `info`, `warn`, or `error`. |
| None | `RUST_LOG` | None | Sets a `tracing` filter and overrides `--log-level`. |

`--allow-root` and `--allow-home` are presence flags. Do not append
`true` or `false`. Their environment variables accept boolean values.
To disable an allowance enabled through the environment, unset its
variable or set it to `false` before running MemLite.

## Storage locations

Default paths follow the operating system's data and cache directories.
On Linux, `XDG_DATA_HOME` and `XDG_CACHE_HOME` override their respective
base directories.

| Platform | Databases | Model cache |
|---|---|---|
| macOS | `~/Library/Application Support/memlite-mcp` | `~/Library/Caches/memlite-mcp` |
| Linux | `~/.local/share/memlite-mcp` | `~/.cache/memlite-mcp` |

The database directory contains one database file per project, plus the
shared worker's lock. The model cache can be shared across projects.
`--db-dir` and `--embedding-cache-dir` override these defaults.

## Project identity

MemLite resolves the project path in this order:

1. The explicit `--project-path` flag or `MEMLITE_PROJECT_PATH` variable.
2. The first usable local `file://` MCP root under `serve`.
3. The current working directory.

An explicit project path skips the MCP roots request. MemLite accepts
localhost authorities and percent-encoded UTF-8 names. It skips remote
hosts, other schemes, malformed escapes, and URI queries or fragments.

MemLite makes relative paths absolute before database hashing. It resolves
parent segments such as `/..` before hashing and validation, through the
filesystem when possible and lexical normalization otherwise.
Filesystem-root and home-directory aliases, including symlinks, require
`--allow-root` or `--allow-home`. Other symlink paths retain their existing
database identity.

Unsupported or malformed roots fall back to the next usable root or the
working directory. Older versions treated some invalid roots as paths.
MemLite does not migrate databases selected by those old strings.

MCP workspace roots are a compatibility fallback. Multi-root hosts may
start server processes with different working directories. Their automatic
fallback may therefore select different databases for the same workspace.
Set one absolute `MEMLITE_PROJECT_PATH` in the client configuration. Use
the same value for direct commands that should share its memories.

For example, add this field to an existing Zed context server entry.
See [client setup](clients.md#zed) for the complete entry.

```json
"env": {
  "MEMLITE_PROJECT_PATH": "/Users/me/Development/my-workspace"
}
```

The explicit value selects the database without requesting deprecated MCP
roots. It can identify a common workspace rather than one repository.
Changing it selects a different database and does not migrate memories.

For file references inside a multi-root database, prefix each path with its
repository root directory name, such as `memlite-mcp/src/config.rs`. See
[references](memory.md#references).

### Database filenames

Each project database uses `<slug>-<hash>.db` inside `--db-dir`. The hash is the
first 16 lowercase hexadecimal characters of the SHA256 of the resolved full
project path, using the same hash input as the previous full-hash scheme.
For example, `/home/user/My Project` selects `my-project-78ce1d89993df547.db`.

The slug is the lowercase ASCII basename. MemLite replaces runs of
punctuation or non-ASCII characters with single hyphens and removes leading
and trailing hyphens. It caps the slug at 80 characters without a trailing
hyphen, and uses `project` when the result is empty or missing. Equal slugs
still hash the entire project path.

## Shared worker election

Every `serve` process starts as a peer. The first process for a canonical
database directory takes `worker.lock` and runs memory operations for
connected MCP sessions. Later processes retain their client-facing MCP
session and proxy tool operations over a mode-0600 Unix socket at
`/tmp/memlite-<hash>/worker.sock`. The hash is the first 32 hexadecimal
characters of the SHA256 of the canonical database directory path. MemLite
creates the socket directory with mode 0700 and verifies its ownership and
permissions before connecting. Symlinks, other owners, and other modes are
rejected without changing the existing path. There is no separate daemon
command or detached process.

The worker keeps one lazy database handle per active project. Read-only
sessions do not open or create absent databases. Embedding models also load
on demand. Projects share a model instance when their selected model,
batch cap, and cache directory match. Different models can coexist in one
worker.

Sessions sharing a project must select the same model, batch cap, and cache
directory. Conflicting settings fail during initialization. Database
metadata also rejects a different model for an already populated project.

When the worker exits, the operating system releases its lock. One
connected proxy becomes the worker and the others reconnect. Client MCP
sessions remain open. The worker does not replay in-flight requests
because a write may already have committed. A request during failover can
fail. Check whether a write took effect before retrying it.

Registry entries use weak references. Database handles and model resources
unload when no MCP session uses them.

## Embedding models

`--embedding-max-batch-size` sets a user cap. The effective inference batch
size is the smaller of that value and the model tier cap. This limits
batches passed to FastEmbed, not total process memory.

| Model | Dimensions | Model files | Tier cap |
|---|---:|---:|---:|
| `bge-small-en-v1.5`, default | 384 | ~128 MiB | 128 |
| `bge-base-en-v1.5` | 768 | ~416 MiB | 64 |
| `bge-large-en-v1.5` | 1024 | ~1276 MiB | 32 |
| `bge-m3` | 1024 | ~2179 MiB | 8 |

Sizes are rounded totals of cached model, tokenizer, and configuration
files from the [benchmark run](benchmarks.md). They exclude runtime memory
and database storage. MemLite selects non-quantized model variants.

The v1.5 models are English-first. `bge-m3` supports 100+ languages but
requires more disk space and inference memory. Its advertised context
window is 8192 tokens. MemLite currently uses FastEmbed's default limit of
512 tokens for every model, so longer inputs are truncated. Split long
memories when the relevant detail would fall beyond that limit.

FastEmbed downloads the selected model on the first embedding operation.
This can be MCP `memory_store`, `memory_search`, or a content-changing
`memory_update`. The corresponding direct commands are `tool store`,
`tool search`, and a content-changing `tool update`. Server startup and
non-embedding operations do not load the model. Later calls work offline
once the files are cached. See [storage locations](#storage-locations).

Each project database records its model on first use. Opening it with a
different model fails because the embeddings would be incompatible.
Switching an existing project to another model requires a fresh database.
See [benchmarks.md](benchmarks.md#embedding-models) for model comparisons.

## Commands

```
memlite [GLOBAL OPTIONS] <COMMAND>
```

| Command | Description |
|---|---|
| [`serve`](#serve) | Runs the MCP server over stdio |
| [`tool`](#tool) | Runs one memory tool directly against the project database |

A subcommand is required; `memlite` alone prints usage and exits.

### `serve`

Runs the MCP server over stdio. Takes no options beyond the
[global options](#global-options).

```sh
memlite serve
```

MCP clients must invoke this subcommand. The server exposes 12 `memory_*`
tools, documented in [tools.md](tools.md). See [client setup](clients.md)
for registration examples.

### `tool`

Runs one memory operation directly against the project database and writes
the result to stdout. CLI command names omit the MCP tools' `memory_`
prefix.

```sh
memlite tool list --limit 10 --include-content false
memlite tool list 55 --format json
memlite tool search "CLI interface plan"
memlite tool update 55 --phase completed
memlite tool delete 55
```

Use the same explicit project path as the MCP client when both should
share memories. Text output is the default. Use JSON for scripts.

```
memlite tool [--format <text|json>] <tool-command>
```

| Option | Default | Description |
|---|---|---|
| `--format <text\|json>` | `text` | Selects output for the command. `text` matches MCP block text and TOON output. `json` emits structured JSON. |

Conventions shared by every subcommand below:

- Required scalar inputs, such as queries and identifiers, are positional.
  Everything else is a kebab-case flag.
- Repeat `--tag`, `--relation id:kind`, and `--ref` to pass multiple values.
- `tool update` provides `--clear-tags`, `--clear-attrs`, `--clear-relations`,
  and `--clear-refs`. These fields are full-set replacements, not merges.
- `--include-content` and `--reversible` require `true` or `false`.
- `tool store` stores one memory per invocation. MCP `memory_store` accepts
  1 to 100 atomic inserts.
- Run `memlite tool <command> --help` for generated usage.

Each subcommand below wraps the identically named MCP tool; see
[tools.md](tools.md) for full parameter semantics, defaults, and behavior.

#### `tool store`

Stores one memory. Wraps [`memory_store`](tools.md#memory_store).

```
memlite tool store <content> --memory-type <type> --summary <summary> [options]
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
| `--relation <id:kind>` | flag, repeatable | Outgoing relation |
| `--ref` | flag, repeatable | External reference |

#### `tool search`

Searches memories by semantic similarity. Wraps
[`memory_search`](tools.md#memory_search).

```
memlite tool search <query> [options]
```

| Argument | Kind | Description |
|---|---|---|
| `query` | positional, required | Natural-language query |
| `--limit` | flag | Maximum number of results |
| `--include-content <BOOL>` | flag | Includes full context and content |
| `--tag` | flag, repeatable | Tag required on every result |
| `--memory-type` | flag | Required memory category |
| `--status`, `--severity`, `--priority`, `--outcome`, `--phase` | flag | Typed attrs filters |
| `--refs-contain` | flag | Required substring in an external reference |

#### `tool list`

Lists memories newest first. Wraps [`memory_list`](tools.md#memory_list).

```
memlite tool list [ids...] [options]
```

| Argument | Kind | Description |
|---|---|---|
| `ids` | positional, repeatable | Exact memory identifiers; missing identifiers are omitted |
| `--limit` | flag | Maximum number of results per page |
| `--offset` | flag | Zero-based pagination offset |
| `--include-content <BOOL>` | flag | Includes full context and content |
| `--tag` | flag, repeatable | Tag required on every result |
| `--memory-type` | flag | Required memory category |
| `--status`, `--severity`, `--priority`, `--outcome`, `--phase` | flag | Typed attrs filters |
| `--refs-contain` | flag | Required substring in an external reference |

#### `tool tags`

Lists tags and occurrence counts. Wraps [`memory_tags`](tools.md#memory_tags).
Takes no arguments beyond `--format`.

```sh
memlite tool tags
```

#### `tool delete`

Deletes one memory. Wraps [`memory_delete`](tools.md#memory_delete).

```
memlite tool delete <id>
```

| Argument | Kind | Description |
|---|---|---|
| `id` | positional, required | Numeric memory identifier |

#### `tool update`

Updates one memory. Wraps [`memory_update`](tools.md#memory_update).

```
memlite tool update <id> [options]
```

| Argument | Kind | Description |
|---|---|---|
| `id` | positional, required | Numeric memory identifier |
| `--content` | flag | Replacement content; triggers re-embedding |
| `--summary` | flag | Replacement summary; blank values are rejected |
| `--tag` | flag, repeatable | Replacement tag |
| `--clear-tags` | presence flag | Clears all tags |
| `--context` | flag | Replacement context; pass an empty string to clear |
| `--confidence` | flag | Replacement confidence score in the range 0 to 1 |
| `--decay-rate` | flag | Replacement per-day decay rate in the range 0 to 1 |
| `--memory-type` | flag | Replacement memory category |
| `--status`, `--severity`, `--priority`, `--outcome`, `--phase` | flag | Replacement typed attrs |
| `--reversible <BOOL>` | flag | Replacement typed attrs |
| `--clear-attrs` | presence flag | Clears all typed attributes |
| `--relation <id:kind>` | flag, repeatable | Replacement relation |
| `--clear-relations` | presence flag | Clears all outgoing relations |
| `--ref` | flag, repeatable | Replacement external reference |
| `--clear-refs` | presence flag | Clears all external references |

#### `tool revalidate`

Resets confidence decay clocks. Wraps
[`memory_revalidate`](tools.md#memory_revalidate).

```
memlite tool revalidate <ids...>
```

| Argument | Kind | Description |
|---|---|---|
| `ids` | positional, repeatable, required, 1 to 100 | Numeric memory identifiers |

#### `tool neighbors`

Lists immediate incoming and outgoing neighbors. Wraps
[`memory_neighbors`](tools.md#memory_neighbors).

```
memlite tool neighbors <id> [options]
```

| Argument | Kind | Description |
|---|---|---|
| `id` | positional, required | Numeric memory identifier |
| `--kind` | flag | Exact relation kind filter |

#### `tool traverse`

Traverses the relation graph breadth first. Wraps
[`memory_traverse`](tools.md#memory_traverse).

```
memlite tool traverse <start_id> [options]
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
memlite tool path <from_id> <to_id> [options]
```

| Argument | Kind | Description |
|---|---|---|
| `from_id` | positional, required | Numeric starting memory identifier |
| `to_id` | positional, required | Numeric destination memory identifier |
| `--max-hops` | flag | Maximum hops, 1 through 10 |

#### `tool hubs`

Lists the most connected memories. Wraps [`memory_hubs`](tools.md#memory_hubs).

```
memlite tool hubs [options]
```

| Argument | Kind | Description |
|---|---|---|
| `--top-n` | flag | Maximum hub count, 1 through 50 |

#### `tool stats`

Shows graph-level statistics. Wraps [`memory_stats`](tools.md#memory_stats).
Takes no arguments beyond `--format`.

```sh
memlite tool stats
```
