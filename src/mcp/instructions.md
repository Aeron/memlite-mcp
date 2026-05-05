Local agent memory store. Embeddings are generated server-side from plain text.

## Tools

- `memory_store`: embed and persist a new memory, returns its numeric id
- `memory_search`: cosine-similarity search, returns memories ordered nearest-first; accepts optional `tags` to pre-filter candidates
- `memory_list`: chronological list with pagination (newest first); accepts optional `tags` to filter
- `memory_tags`: lists all tags in use with occurrence counts, ordered by frequency
- `memory_update`: update content/summary/tags in-place; re-embeds when content changes
- `memory_delete`: remove a memory by its identifier

## Workflow

### Before storing

Call `memory_tags` to see what tags already exist and reuse them where they fit; inventing
synonyms fragments the taxonomy over time. Then call `memory_search`. If a very close match
exists (distance < 0.15), update that memory instead of creating a duplicate. Store only
when no close match is found.

### Searching vs listing

Use `memory_search` when you have a concept or topic in mind since it returns semantically
relevant results regardless of age. Use `memory_list` only for chronological review
(e.g. "what did I store recently?").

### Understanding distance scores

- distance == 0.0 : identical vector (exact duplicate)
- distance < 0.15 : very close (likely same concept)
- distance < 0.40 : related (same domain)
- distance > 0.70 : unrelated (different topics)

## `summary` field

Provide summary when content is:
- longer than 2-3 sentences
- technical jargon that won't be meaningful in a search snippet
- a code block or structured data

Summary is shown in list views and used to decide whether to read full content.
Skip summary for short, self-describing facts.

## `tags` field

Tags are for grouping and filtering. Use them when the topic is clear.
Keep tags lowercase, singular where natural: "auth" not "Authentication".
Example sets: ["architecture", "database"], ["bug", "auth"], ["decision", "api-design"].
Skip tags for one-off facts that don't belong to a named domain.

The `memory_search` and `memory_list` both accept an optional `tags` array. All supplied tags
must be present on a memory for it to be included. Combine with a query in `memory_search`
to narrow semantic search to a specific category (e.g. find the most relevant bug reports).

## Updating vs deleting

Prefer update when the core identity of the memory is the same but details changed.
Delete only when the memory is fully obsolete or factually wrong with no salvageable parts.
Updating content triggers re-embedding automatically.
Pass summary as empty string ("") to clear it.
