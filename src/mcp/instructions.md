Local agent memory store. Server-side embeddings from plain text.

## Tools

- `memory_store`: embed + persist; returns id. Optional: `confidence` ([0,1], default 1.0), `decay_rate` (per-day, default 0.0), `memory_type` (`user`/`project`/`reference`/`task`/`finding`/`issue`/`decision`/`plan`), `status` (`open`/`resolved`/`wontfix`/`deferred`), `severity` (`critical`/`high`/`medium`/`low`), `relations` (JSON object `{"<id>": "<kind>"}`, e.g. `{"7": "caused_by"}`)
- `memory_store_batch`: batch store; `items: [...]` (max 100), same fields as `memory_store`; single-pass embed, atomic insert; returns `[{"id": ...}]` in input order. Prefer over repeated `memory_store` for 3+ memories
- `memory_search`: cosine + keyword re-rank; nearest-first with `effective_confidence` and `age_days`. Pre-filter: `tags`, `memory_type`, `status`, `severity`
- `memory_list`: newest-first with pagination. Filter: `tags`, `memory_type`, `status`, `severity`; returns raw `confidence`, `decay_rate`, `last_seen_at`
- `memory_tags`: all tags with occurrence counts, frequency-ordered
- `memory_update`: update content/summary/tags/context/confidence/decay_rate/memory_type/status/severity/relations in-place; re-embeds on content change; resets decay clock; pass `""` to clear summary/context/memory_type/status/severity; pass `{}` to clear relations
- `memory_delete`: remove by id
- `memory_revalidate`: resets decay clock (`last_seen_at` = now) on a list of ids without changing any other field; returns count updated. Use when confirming one or more memories accurate in same session
- `memory_neighbors`: immediate in/out neighbors; optional `kind` filter; `direction` ("out"/"in"/"both")
- `memory_traverse`: BFS from `start_id`; `depth` (default 2, max 5), optional `kind`, `direction`
- `memory_path`: shortest directed path; `from_id`, `to_id`, optional `max_hops` (default 6)
- `memory_hubs`: top-N by total degree; optional `top_n` (default 10)
- `memory_stats`: graph counts; type distribution, relation kinds

## Workflow

### Before investigating

Call `memory_search` before any investigation, debugging, or implementation. Query: symptom, error, component, or concept. May resolve immediately or surface prior decisions. Skipping risks full-cost rediscovery.

### After completing work

Before finishing, store anything saving meaningful future effort. Don't wait to be asked. Good candidates:

- Decision: what chosen, what rejected, why
- Non-obvious fact: quirk, undocumented behavior, workaround, env detail
- Bug diagnosed: root cause + fix, not just symptom
- Constraint: anything ruling out or shaping future approaches
- User preference or project convention: naming, tooling, process
- Significant research: if it cost time once, store it

Plain mechanical steps don't need storing (e.g., "ran tests"). Test: would future session benefit without rediscovery?

### Before storing

Call `memory_tags` to reuse existing tags; inventing synonyms fragments taxonomy. Then `memory_search`. Distance < 0.15: update, not create. Store only when no close match found.

### Searching vs listing

`memory_search` for concept/topic; semantic, age-agnostic. `memory_list` only for chronological review (e.g., "what stored recently?").

### Graph navigation

Use when you have ids and want to explore connections, not discover from scratch:

- `memory_neighbors`: immediate links from/to a node
- `memory_traverse`: BFS cluster from a seed node
- `memory_path`: how two nodes connect; trace cause/decision chains
- `memory_hubs`: most-connected nodes; good orientation on unfamiliar project
- `memory_stats`: graph shape overview

### How `memory_search` ranks

Two stages: cosine selects candidate pool (~4x return count), keyword overlap re-ranks within it. Pre-filters (`tags`, `memory_type`, `status`, `severity`) exclude before both stages. Result missing: drop filters first; still absent, rephrase with different vocabulary (semantic = embedding-based, not keyword-based).

### Distance scores

- 0.0: identical vector
- < 0.15: very close (likely same concept)
- < 0.40: related (same domain)
- > 0.70: unrelated

### `effective_confidence` and `age_days`

`effective_confidence` = base confidence - accumulated decay since last validation, clamped [0,1].

- >= 0.8: reliable
- 0.5-0.8: mild caution; re-verify if decision significant
- < 0.5: potentially stale; confirm before use

Confirmed accurate? Call `memory_revalidate([id])`. Multiple: pass all ids in one call. If also changing a field, `memory_update` resets clock too.

## Fields

### `summary`

Provide when content > 2-3 sentences, uses jargon, or is structured data. Shown in list views; helps decide whether to read full content. Skip for short, self-describing facts. Pass `""` to clear.

### `context`

Record the *why*: reasoning, constraints, trade-offs, broader situation. Not a summary. Use for decisions, architectural choices, anything where rationale matters later. Skip for plain self-contained facts. Pass `""` to clear.

### `confidence`

Float [0.0, 1.0]. Provide when info uncertain:
- Speculative/inferred: 0.5-0.7
- Believed correct, unverified: 0.8-0.9
- Confirmed: 1.0 (default)

### `decay_rate`

Float [0.0, 1.0]. Reduces `effective_confidence` by amount per day since last validation:
- 0.0 (default): no decay; stable facts, decisions, conventions
- 0.005-0.01: slow; implementation details drifting over months
- 0.02-0.05: moderate; env state, dependency versions, workarounds
- 0.1+: fast; time-sensitive (e.g., "build currently broken")

### `tags`

Grouping and filtering. Lowercase, singular where natural: `"auth"` not `"Authentication"`. Examples: `["architecture", "database"]`, `["bug", "auth"]`. Skip for one-off facts with no named domain.

Both `memory_search` and `memory_list` accept optional `tags`. All must match (hard AND pre-filter; not a relevance boost). Fewer tags = broader pool; result missing, drop tags first. Combine with query to narrow domain.

### `memory_type`

| Value | Use for |
|---|---|
| `user` | Preferences, identity, working style; personal opinions on work quality or agent approach |
| `project` | Conventions, tech stack, architecture, constraints |
| `reference` | Domain knowledge, docs, external facts, API details |
| `task` | Things to track, follow up, or complete |
| `finding` | Observation, evaluation, or risk; covers audit results, review findings, quality assessments, and retrospective notes; not yet a tracked task |
| `issue` | Known defect or bug |
| `decision` | Architectural or design decision record: what was chosen, what rejected, and why |
| `plan` | High-level plan or phased roadmap; use `relations` to link to spawned tasks and decisions; apply moderate `decay_rate` (0.01-0.02) |

Set when memory fits one category; skip if multi-category or unclear. Pass `""` to `memory_update` to clear. Both tools accept `memory_type` as filter; combine with `tags` and `status` for precise retrieval.

Feedback disambiguation: personal opinions on agent approach or working style -> `user`; quality or correctness observations about the work itself -> `finding` or `issue`.

### `status`

Lifecycle: `open` / `resolved` / `wontfix` / `deferred`. For task/issue/finding; omit general knowledge. Don't delete resolved memories; update status instead. Pass `""` to clear. Use `status` for lifecycle (fixed or not), `decay_rate` for time-based staleness.

### `severity`

Priority: `critical` / `high` / `medium` / `low`. For finding/issue/task; omit general knowledge. Pass `""` to clear. Combine with `status` to filter e.g. open critical issues.

### `relations`

Outgoing directed links: JSON object `{"<target_id>": "<kind>"}`. Kind is free-form; common: `caused_by`, `causes`, `blocks`, `depends_on`, `supersedes`, `related_to`, `duplicate_of`. Links one-way; reverse traversal via `memory_neighbors`. `memory_update` replaces full set; pass `{}` to clear.

## Updating vs deleting

Revalidating without field changes: `memory_revalidate` (single or list of ids). `memory_update` also resets clock when bundling a field change.

Prefer update when core identity same but details changed: decision revised, fix turned incomplete, constraint shifted, new info extends. Update proactively when current work makes memory stale.

Delete only when fully obsolete or factually wrong with nothing salvageable. Content update triggers re-embedding automatically.
