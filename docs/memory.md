# Memory Model

This document describes the shape of a memory record and the rules that
govern it. It applies uniformly to the MCP tools (see
[tools.md](tools.md)) and the direct CLI (see
[configuration.md](configuration.md)); both are thin interfaces over the
same underlying store.

Each memory is a record with required `content`, `summary`, and `memory_type`
fields. Optional fields are `tags`, `context`, `confidence`, `decay_rate`,
`attrs`, `relations`, and `refs`. The sections below describe the fields that
carry structure.

## Memory types

The required `memory_type` field classifies a memory's content, enabling
targeted filtering in `memory_list` and `memory_search`. Classify the result,
not the activity that produced it or the person who reported it. Use specific
types for assessments, defects, choices, actions, and roadmaps even when they
concern this codebase; `project` is for general codebase facts and shared rules.

| Type | Stores | Example |
|---|---|---|
| `user` | Personal preferences, identity, or working style; shared repository rules belong to `project` | "Prefers concise replies" |
| `project` | General codebase facts and shared rules: stack, current architecture, constraints; choice rationale belongs to `decision` | "This repository uses SQLite" |
| `reference` | Reusable external or domain knowledge: docs, API details, research synthesis; general codebase facts belong to `project` | "SQLite WAL allows readers alongside a writer" |
| `task` | Concrete action or follow-up, including unfinished research; multi-step roadmaps belong to `plan` | "Benchmark SQLite write throughput" |
| `finding` | Assessment or risk: comparative evaluations, audit/review results, suspected defects; confirmed defects belong to `issue` | "Write contention may limit throughput" |
| `issue` | Confirmed defect or bug, including root cause and fix; remediation actions belong to `task` | "Concurrent writes fail because retries are missing" |
| `decision` | Recorded choice: chosen or rejected options and rationale; a recommendation alone belongs to `finding` | "Chose SQLite over a remote database to keep storage local" |
| `plan` | Multi-step strategy or phased roadmap; use `relations` to link to tasks and decisions; a single action belongs to `task` | "Measure contention, add retries, then validate under load" |

Research is an activity, not a type. Store reusable knowledge as `reference`,
codebase facts as `project`, evaluations as `finding`, and recorded choices as
`decision`. Research still to do is `task`; a research roadmap is `plan`.
An optional `research` tag groups results across types. Put source URLs or
file paths in `refs`. Split mixed reports when separate recall would help.

Feedback is a source, not a type. Personal preferences belong to `user`, even
when expressed as feedback; shared repository rules belong to `project`, even
when reported by the user. Work evaluations belong to `finding`; confirmed
bugs belong to `issue`. Recommendations alone are `finding`, not `decision`.

Keep problem records as `finding` or `issue` when adding follow-up work; store
remediation actions separately as linked `task` records when useful. A fact
is not a `finding` merely because it was discovered during investigation.
Use `confidence` for uncertain facts. Completion does not change type:
resolved defects stay `issue`, completed actions stay `task`.

## Typed attrs

`attrs` is an optional JSON object for type-specific fields.

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

Each memory carries a `confidence` score (base trust, 0.0–1.0) and an
optional `decay_rate` (per-day reduction, 0.0–1.0). At query time the server
computes `effective_confidence = confidence - decay_rate * days_since_last_seen`,
clamped to [0, 1]. `memory_search` results include this derived value
together with readable RFC 3339 timestamps and an optional compact
`freshness` warning.

Stored confidence and decay values must be finite. Finite inputs are clamped
to [0, 1]; `NaN` and infinities are rejected without changing stored data.

Set `confidence < 1.0` when storing uncertain or speculative information.
Set `decay_rate > 0.0` for time-sensitive facts that naturally become stale
(e.g. `0.01` decays ~1% per day). Calling `memory_update` on any field resets
`last_seen_at` to now, restarting the decay clock. Use `memory_revalidate`
to confirm one or more memories are still accurate without changing their
content.

## Relations

Memories can carry directed outgoing links via the `relations` field: a JSON
object mapping a target memory identifier (string key) to a free-form
relation kind:

```json
{"7": "caused_by", "12": "blocks"}
```

Links are one-way; the target memory is unmodified. Every target identifier
must name an existing memory. Common kinds: `caused_by`, `causes`, `blocks`,
`depends_on`, `supersedes`, `related_to`, `duplicate_of`. Kind strings are
free-form; pick a label and use it consistently within a project.
`memory_update` replaces the full set; pass `{}` to clear all links.

## References

Memories can carry an optional `refs` field: an array of free-form strings
pointing to external resources related to the memory:

```json
["src/db/store.rs", "MemoryStore::insert", "https://github.com/foo/bar/issues/42"]
```

Useful kinds of entries: project-relative file paths, fully-qualified symbol
names, URLs to docs or issues, ticket identifiers. In a single-project
database, use paths relative to that project, such as `src/db/store.rs`. In a
multi-project database, prefix paths with the repository root directory name,
such as `memlite-mcp/src/db/store.rs`. Do not use absolute paths.

References remain exactly as provided because root prefixes disambiguate files
from different repositories. MemLite does not rewrite old or new values.
`memory_update` replaces the full set; pass `[]` to clear.

Both `memory_search` and `memory_list` accept an optional `refs_contain`
string that filters results to memories whose `refs` array contains the
given literal substring in any entry. Query file references with the same
single-project or root-prefixed form used when storing them.

## Knowledge graph

Relations are stored in a normalized `memory_relations(from_id, to_id, kind)`
table with an index on `to_id` for efficient reverse lookups. All write
operations (`memory_store`, `memory_update`, `memory_delete`) keep this
table in sync with the `relations` JSON column.

Five tools operate on this table: `memory_neighbors`, `memory_traverse`,
`memory_path`, `memory_hubs`, and `memory_stats`. All five operate on the
full global graph of the project DB. Multi-query graph reads use one isolated
read snapshot, so concurrent writes cannot mix graph versions within a result.
