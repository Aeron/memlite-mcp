# CLI reference

Run `memlite serve` for the MCP server or `memlite tool <command>`
for direct memory management. Both use the same project database.

See [configuration](configuration.md) for storage, project identity, shared
workers, and embedding models. See [client setup](clients.md) to connect an
agent and [tools.md](tools.md) for tool behavior.

## Global options

Shared options accept both CLI flags and environment variables. Flags take
precedence. Global flags work before or after nested commands under both
`serve` and `tool`. Run `memlite --help` for generated usage.

| Flag | Environment variable | Default | Description |
|---|---|---|---|
| `--project-path` | `MEMLITE_PROJECT_PATH` | First usable MCP workspace root under `serve`, then the working directory. The working directory under `tool`. | Project identity, resolved to an absolute path before database selection. |
| `--db-dir` | `MEMLITE_DB_DIR` | OS data directory / `memlite-mcp` | Directory for per-project databases. See [storage locations](configuration.md#storage-locations). |
| `--allow-root` | `MEMLITE_ALLOW_ROOT` | `false` | Allows `/` as the project path. This shares one database across the filesystem. |
| `--allow-home` | `MEMLITE_ALLOW_HOME` | `false` | Allows the home directory as the project path. This can combine unrelated projects. |
| `--embedding-model` | `MEMLITE_EMBEDDING_MODEL` | `bge-small-en-v1.5` | Selects an [embedding model](configuration.md#embedding-models). |
| `--embedding-max-batch-size` | `MEMLITE_EMBEDDING_MAX_BATCH_SIZE` | `128` | Caps inference batches. The model tier can impose a lower cap. |
| `--embedding-cache-dir` | `MEMLITE_EMBEDDING_CACHE_DIR` | OS cache directory / `memlite-mcp` | Overrides the model cache location. |
| `--log-file` | `MEMLITE_LOG_FILE` | None | Writes logs to this file instead of stderr. |
| `--log-level` | `MEMLITE_LOG_LEVEL` | `info` | Sets the minimum log level when `RUST_LOG` is absent. Accepts `trace`, `debug`, `info`, `warn`, or `error`. |
| None | `RUST_LOG` | None | Sets a `tracing` filter and overrides `--log-level`. |

`--allow-root` and `--allow-home` are presence flags. Do not append
`true` or `false`. Their environment variables accept boolean values.
To disable an allowance enabled through the environment, unset its
variable or set it to `false` before running MemLite.

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
