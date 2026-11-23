# Memory model

The MCP tools and direct CLI use the same memory store and follow the same
rules. See [tools.md](tools.md) for the MCP tools and
[cli.md](cli.md) for the CLI.

Each memory requires `content`, `summary`, and `memory_type`. Optional fields
are `tags`, `context`, `confidence`, `decay_rate`, `attrs`, `relations`, and
`refs`.

## Memory identifiers

Each memory has a stable numeric identifier within its project database.
Deletion does not make that identifier available for a later memory, and
SQLite maintenance preserves surviving identifiers.

## Memory types

The required `memory_type` field classifies a memory's content.
`memory_list` and `memory_search` can filter by this type. Classify the result,
not the activity that produced it or the person who reported it. Use specific
types for assessments, defects, choices, actions, and roadmaps even when they
concern this codebase. Use `project` for general codebase facts and shared rules.

| Type | Stores | Example |
|---|---|---|
| `user` | Use this type for personal preferences, identity, or working style. Store shared repository rules as `project`. | "Prefers concise replies" |
| `project` | Use this type for general codebase facts and shared rules, including the stack, current architecture, and constraints. Store the rationale for a choice as `decision`. | "This repository uses SQLite" |
| `reference` | Use this type for reusable external or domain knowledge, including documentation, API details, and research synthesis. Store general codebase facts as `project`. | "SQLite WAL allows readers alongside a writer" |
| `task` | Use this type for a concrete action or follow-up, including unfinished research. Store multi-step roadmaps as `plan`. | "Benchmark SQLite write throughput" |
| `finding` | Use this type for an assessment or risk, including comparative evaluations, audit and review results, and suspected defects. Store confirmed defects as `issue`. | "Write contention may limit throughput" |
| `issue` | Use this type for a confirmed defect or bug, including its root cause and fix. Store remediation actions as `task`. | "Concurrent writes fail because retries are missing" |
| `decision` | Use this type for a recorded choice, including chosen or rejected options and the rationale. Store a recommendation alone as `finding`. | "Chose SQLite over a remote database to keep storage local" |
| `plan` | Use this type for a multi-step strategy or phased roadmap. Use `relations` to link to tasks and decisions. Store a single action as `task`. | "Measure contention, add retries, then validate under load" |

Research is an activity, not a type. Store reusable knowledge as `reference`,
codebase facts as `project`, evaluations as `finding`, and recorded choices as
`decision`. Store unfinished research as `task` and a research roadmap as
`plan`. Use an optional `research` tag to group results across types. Put
source URLs or file paths in `refs`. Split mixed reports when separate recall
would help.

Feedback is a source, not a type. Store personal preferences as `user`, even
when someone expresses them as feedback. Store shared repository rules as
`project`, even when the user reports them. Store work evaluations as
`finding` and confirmed bugs as `issue`. Store recommendations alone as
`finding`, not `decision`.

Keep problem records as `finding` or `issue` when adding follow-up work.
Store remediation actions separately as linked `task` records when useful.
Discovering a fact during investigation does not make it a `finding`.
Use `confidence` for uncertain facts. Completion does not change the type:
resolved defects stay `issue`, and completed actions stay `task`.

## Typed attrs

Use the optional `attrs` JSON object for type-specific fields.

| Field | Values | Most useful for |
|---|---|---|
| `status` | `open`, `resolved`, `wontfix`, `deferred` | task, issue, finding |
| `severity` | `critical`, `high`, `medium`, `low` | issue, finding |
| `priority` | `urgent`, `high`, `medium`, `low` | task |
| `outcome` | `accepted`, `rejected`, `superseded`, `deferred` | decision |
| `reversible` | `true`, `false` | decision |
| `phase` | `draft`, `approved`, `active`, `completed` | plan |

Do not delete resolved memories; they are useful history. Set `attrs.status`
to `resolved` instead and leave the record intact for future sessions.
`memory_search` and `memory_list` accept `status`, `severity`, `priority`,
`outcome`, and `phase` as filters.

## Trust and decay

Each memory carries a `confidence` score for base trust and an optional
`decay_rate` for the per-day reduction. Both values range from 0.0 to 1.0.
At query time, the server computes
`effective_confidence = confidence - decay_rate * days_since_last_seen`
and clamps the result to [0, 1]. `memory_search` results include this derived
value, readable RFC 3339 timestamps, and an optional compact `freshness` warning.

Stored confidence and decay values must be finite. The server clamps finite
inputs to [0, 1] and rejects `NaN` and infinities without changing stored data.

Set `confidence < 1.0` when storing uncertain or speculative information.
Set `decay_rate > 0.0` for time-sensitive facts that become stale.
For example, `0.01` reduces confidence by one percentage point per day.
Calling `memory_update` on any field resets `last_seen_at` to now and restarts
the decay clock. Use `memory_revalidate` to confirm that one or more memories
are still accurate without changing their content.

## Relations

The optional `relations` JSON object stores directed outgoing links. Each
entry maps a target memory identifier, written as a string key, to a
free-form relation kind:

```json
{"7": "caused_by", "12": "blocks"}
```

Links are one-way and do not modify the target memory. Every target
identifier must name an existing memory. Common kinds include `caused_by`,
`causes`, `blocks`, `depends_on`, `supersedes`, `related_to`, and `duplicate_of`.
Kind strings are free-form. Pick a label and use it consistently within a
project. `memory_update` replaces the full set. Pass `{}` to clear all links.

## References

The optional `refs` array stores free-form strings that point to external
resources related to the memory:

```json
["src/db/store.rs", "MemoryStore::insert", "https://github.com/foo/bar/issues/42"]
```

Use references for project-relative file paths, fully qualified symbol
names, URLs to documentation or issues, and ticket identifiers. In a
single-project database, use paths relative to that project, such as
`src/db/store.rs`. In a multi-project database, prefix paths with the
repository root directory name, such as `memlite-mcp/src/db/store.rs`.
Do not use absolute paths.

MemLite preserves references exactly as provided and does not rewrite old or
new values. Root prefixes distinguish files from different repositories.
`memory_update` replaces the full set. Pass `[]` to clear all references.

Both `memory_search` and `memory_list` accept an optional `refs_contain`
string. This filter selects memories with at least one `refs` entry that
contains the given literal substring. Query file references with the same
single-project or root-prefixed form you used when storing them.

## Knowledge graph

The store keeps relations in a normalized
`memory_relations(from_id, to_id, kind)` table. An index on `to_id` supports
efficient reverse lookups. All write operations keep this table in sync with
the `relations` JSON column: `memory_store`, `memory_update`, and `memory_delete`.

Five tools read this table: `memory_neighbors`, `memory_traverse`,
`memory_path`, `memory_hubs`, and `memory_stats`. All five use the entire
global graph of the project database. Graph reads that need multiple queries use
one isolated read snapshot, so concurrent writes cannot mix graph versions
within a result.
