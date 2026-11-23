# Configuration

This guide covers storage, project identity, shared workers, and embedding
models. See the [CLI reference](cli.md) for commands, flags, and environment
variables, or [client setup](clients.md) to connect an agent.

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

### Database permissions

On macOS and Linux, both direct commands and the shared worker create missing
database directories with mode `0700` and new database files with mode `0600`.
This does not depend on a restrictive process umask. SQLite creates new WAL,
SHM, and rollback-journal files using the database file's mode, so new private
databases also have private sidecars. Missing directory ancestors are created
with `0700`; existing ancestors are never chmodded.

Directory symlinks are supported and resolved for shared worker identity;
MemLite does not change the target directory's existing mode. Dangling
directory symlinks fail. Database and sidecar paths that are symlinks or
non-regular files are rejected before SQLite opens them, including dangling
file symlinks. Use trusted parent directories: these path checks do not defend
against concurrent path replacement by someone who can write those directories.
Unix modes do not provide encryption or manage filesystem ACLs, backups, model
caches, or log-file permissions.

A schema-version preflight rejects databases from newer MemLite versions before
applying persistent pragmas such as WAL mode. Migrations recheck the version
inside their write transaction before changing schema or model metadata.

### Durability

MemLite uses SQLite WAL mode with `synchronous=normal`. Committed writes
survive a MemLite process crash. An operating-system crash or power loss can
lose recent commits. WAL protects transaction atomicity and database
consistency under SQLite's filesystem and storage assumptions. It does not
guarantee that every acknowledged write has reached disk. Keep backups for
data you cannot afford to lose.

### Backup and restore

Stop all database access before copying files. Copying only a live `.db` file
can miss committed writes still in its `-wal` file.

To back up:

1. Disable automatic client restarts. Stop all `memlite serve` processes that
   share the database directory and wait for every `memlite tool` command to
   finish. Include proxy peers, which can take over when the worker exits.
2. Copy each `.db` file and any matching `-wal`, `-shm`, and `-journal` sidecars
   into a new owner-only backup directory outside the live database directory.
   Keep the original filenames and copy all files from the same stopped
   snapshot. Do not delete sidecars. Exclude `worker.sock` and `worker.lock`,
   which MemLite recreates.
3. Compare source and backup file checksums before restarting MemLite.

To restore:

1. Stop all MemLite processes using the target directory. Save its current
   files separately for rollback. Restore the complete snapshot into an empty
   directory owned by the current user. Use mode `0700` for the directory and
   `0600` for database files and sidecars. Never overlay a database onto
   unrelated sidecars.
2. Use a compatible MemLite version, the original absolute `--project-path`,
   and the original `--embedding-model`. Point direct commands at the restored
   directory with `--db-dir`. Changing the project path selects a different
   database. It does not move the restored memories.
3. Leave restored sidecars in place for SQLite recovery on first open. Compare
   `tool stats` counts and `tool list` records with the expected backup contents.
   Run `tool search` for a known memory. Reconnect clients only after these
   checks pass, with the restored directory and original model.

Test a restore in an isolated directory before relying on a backup. Back up
the embedding cache too if recovery must work offline. Otherwise, MemLite can
download the model files again. Backups cannot recover writes lost before the
snapshot.

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
`<db-dir>/worker.sock`, beside `worker.lock`. No temporary socket directory
or path hash is needed. The canonical database directory must belong to
the current effective user and must not allow group or other write access.
Modes such as 0700 and 0755 are accepted without changing permissions.
Both ends verify peer credentials before exchanging protocol data, including
on systems that do not enforce socket permission bits. One worker serves
one user per database directory; different users must use separate directories.
There is no separate daemon command or detached process.

The canonical socket path must fit the operating system's Unix socket limit
(103 bytes on macOS, 107 on Linux). Longer paths fail immediately with a
request to use a shorter `--db-dir`. A shorter symlink alias does not help
because the database directory is canonicalized.

When upgrading from temporary socket paths, stop all existing `serve`
processes sharing the database directory before starting upgraded processes.
Old and new peers share the lock but cannot connect to each other's sockets.

The worker keeps one lazy database handle per active project. Read-only
sessions do not open or create absent databases. Embedding models also load
on demand. Projects share a model instance when their selected model,
batch cap, and cache directory match. Different models can coexist in one
worker.

Sessions sharing a project must select the same model, batch cap, and cache
directory. Conflicting settings fail during initialization. Database
metadata also rejects a different model for an already populated project.

On normal shutdown, the worker removes its socket before the operating
system releases its lock. The lock file stays in place so peers always
lock the same file. Forced termination can leave a stale socket; the next
elected worker replaces it while holding the lock. Test and benchmark
sockets live inside their managed temporary database directories and are
removed with those directories.

When the worker exits, one connected proxy becomes the worker and the
others reconnect. Client MCP sessions remain open. The worker does not replay in-flight requests
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
