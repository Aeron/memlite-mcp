Local agent memory store. Embeddings are generated server-side from plain text.

## Tools

- `memory_store`: embed and persist a new memory, returns its numeric id; accepts optional `confidence` (trust score in [0, 1], default 1.0) and `decay_rate` (per-day decay, default 0.0)
- `memory_search`: two-stage search (cosine similarity then keyword re-ranking), returns memories ordered nearest-first with `effective_confidence` and `age_days` per result; accepts optional `tags` to pre-filter candidates
- `memory_list`: chronological list with pagination (newest first); accepts optional `tags` to filter; returns raw `confidence`, `decay_rate`, `last_seen_at` fields
- `memory_tags`: lists all tags in use with occurrence counts, ordered by frequency
- `memory_update`: update content/summary/tags/context/confidence/decay_rate in-place; re-embeds when content changes; always resets the decay clock
- `memory_delete`: remove a memory by its identifier

## Workflow

### Before investigating

Call `memory_search` before starting any investigation, debugging session, or implementation
task. Use the symptom, error message, component name, or concept as the query. A stored
memory may resolve the issue immediately or surface prior decisions that constrain the
solution. Skipping this step risks re-discovering known information at full cost.

### After completing work

Before finishing any task, review what was produced or discovered and store anything that
would save meaningful effort in a future session. Do not wait to be asked. Good candidates:

- A decision was made or confirmed — record what was chosen, what was rejected, and why
- A non-obvious fact was discovered — a quirk, undocumented behavior, workaround, or
  environment-specific detail that took effort to find
- A bug was diagnosed — root cause and fix, not just the symptom
- A constraint was identified — something that will rule out or shape future approaches
- A user preference or project convention was stated — naming style, tooling choice,
  process rule, anything the user would expect to be remembered
- Something took significant research to figure out — if it cost time once, store it so
  it costs nothing next time

Plain mechanical steps do not need storing (e.g. "ran the tests"). The test is: would a
future session benefit from knowing this without having to rediscover it?

### Before storing

Call `memory_tags` to see what tags already exist and reuse them where they fit; inventing
synonyms fragments the taxonomy over time. Then call `memory_search`. If a very close match
exists (distance < 0.15), update that memory instead of creating a duplicate. Store only
when no close match is found.

### Searching vs listing

Use `memory_search` when you have a concept or topic in mind since it returns semantically
relevant results regardless of age. Use `memory_list` only for chronological review
(e.g. "what did I store recently?").

### Understanding `distance` scores

- distance == 0.0 : identical vector (exact duplicate)
- distance < 0.15 : very close (likely same concept)
- distance < 0.40 : related (same domain)
- distance > 0.70 : unrelated (different topics)

### Understanding `effective_confidence` and `age_days`

`memory_search` results include `effective_confidence` (base confidence minus
accumulated decay since last validation, clamped to [0, 1]) and `age_days`
(days since creation).

- effective_confidence >= 0.8 : treat as reliable
- effective_confidence 0.5-0.8 : treat with mild caution; re-verify if the decision is significant
- effective_confidence < 0.5 : treat as potentially stale; confirm before relying on it

If you confirm that a low-confidence memory is still accurate, call `memory_update`
on it (updating any field, including setting `confidence` to its current value)
to reset `last_seen_at` and restart the decay clock.

## `summary` field

Provide summary when content is:
- longer than 2-3 sentences
- technical jargon that won't be meaningful in a search snippet
- a code block or structured data

Summary is shown in list views and used to decide whether to read full content.
Skip summary for short, self-describing facts.

## `context` field

Provide context when you want to record the *why* behind a memory: the reasoning that led
to a decision, the constraints that shaped it, the trade-offs considered, or the broader
situation that makes the content significant. Context is not a repetition of the content
and not a compressed summary; it answers questions like:

- "Why was this approach chosen over the alternatives?"
- "What constraint or incident prompted this?"
- "Who or what does this affect, and how?"

Skip context for plain facts that are self-contained (e.g., "the API key is stored in
`~/.config/myapp`"). Use it for decisions, architectural choices, and anything where the
rationale will matter later.

## `confidence` field

Provide `confidence` (a float in [0.0, 1.0]) when the information is not fully
certain. Examples:

- Speculative or inferred knowledge: 0.5-0.7
- A fact you believe is correct but have not verified: 0.8-0.9
- Confirmed, well-established information: 1.0 (default)

Skip `confidence` (or leave at 1.0) for facts you are sure about.

## `decay_rate` field

Provide `decay_rate` (a float in [0.0, 1.0]) when the memory describes something
that naturally becomes stale over time. It reduces `effective_confidence` by that
amount per day since last validation.

Guide:
- 0.0 (default): no decay; memory stays as confident as when it was stored
- 0.005-0.01: slow decay; relevant for implementation details that may drift over months
- 0.02-0.05: moderate decay; relevant for environment state, dependency versions, or workarounds
- 0.1+: fast decay; relevant for time-sensitive facts like "the build is currently broken"

Skip `decay_rate` (or leave at 0.0) for stable facts like decisions, conventions,
architectural choices, or anything unlikely to change.

## `tags` field

Tags are for grouping and filtering. Use them when the topic is clear.
Keep tags lowercase, singular where natural: "auth" not "Authentication".
Example sets: ["architecture", "database"], ["bug", "auth"], ["decision", "api-design"].
Skip tags for one-off facts that don't belong to a named domain.

The `memory_search` and `memory_list` both accept an optional `tags` array. All supplied tags
must be present on a memory for it to be included. Combine with a query in `memory_search`
to narrow semantic search to a specific category (e.g. find the most relevant bug reports).

## Updating vs deleting

Calling `memory_update` also resets the decay clock (`last_seen_at` = now),
extending the memory's effective lifetime. Use this as a revalidation step: if
you confirm a memory is still accurate during a session, update it (even just
set `confidence` to its current value) to signal that it has been verified.

Prefer update when the core identity of the memory is the same but details changed.
Common triggers: a decision was revised, a fix turned out incomplete, a constraint relaxed
or tightened, or new information extends what was already stored. Update proactively
whenever work in the current session makes a stored memory stale or incomplete.

Delete only when the memory is fully obsolete or factually wrong with no salvageable parts.
Updating content triggers re-embedding automatically.
Pass summary as empty string ("") to clear it.
Pass context as empty string ("") to clear it.
