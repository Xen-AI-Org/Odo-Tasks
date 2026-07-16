## Context

Odo exposes one MCP server over stdio and stateful Streamable HTTP. Tool schemas and discovery records are generated from Rust types and macros, while behavior is implemented against the shared SQLite workspace. The conformance gaps span transport configuration, generated metadata, runtime validation, and response shaping, so the fix must be verified at the JSON-RPC boundary rather than only through unit tests.

## Goals / Non-Goals

**Goals:**

- Make advertised schemas, descriptions, resource metadata, and prompt metadata match runtime behavior.
- Produce deterministic validation and pagination results for strict clients.
- Prevent absolute local paths from appearing in normal tool responses.
- Remove the empty SSE priming record without rewriting or buffering the response stream.
- Preserve revision safety, cycle safety, and local-first storage.
- Keep a repeatable protocol-level regression suite in the repository.

**Non-Goals:**

- Changing frontend navigation or adding MCP control over the GUI.
- Making backup files remotely downloadable.
- Replacing rmcp or adding a second HTTP implementation.
- Supporting arbitrary folder parents or weakening folder-cycle validation.

## Decisions

1. Disable rmcp's SSE retry priming event with `with_sse_retry(None)`. The empty `data:` record comes from rmcp's stateful priming event; disabling that optional retry hint removes the record while preserving stateful sessions, real JSON-RPC SSE events, and keep-alive comments. A custom streaming body filter was rejected because it would need chunk-boundary state and could corrupt valid event data.

2. Treat generated MCP discovery as a tested public contract. Rust argument structs remain the schema source, but field constraints and descriptions are explicit, and the protocol suite inspects `tools/list`, `resources/list`, `resources/templates/list`, and `prompts/list` exactly as a client sees them.

3. Define `limit=0` as an empty terminal page. List/search tools return no items, `hasMore=false`, and `nextOffset=null`; this avoids returning one item or producing a non-advancing cursor.

4. Validate note titles in one shared helper. Titles are trimmed only for validation, preserve the caller's original text, require at least one non-whitespace character, and allow at most 1,000 Unicode scalar values. Create, update, and atomic batch paths all use the same rule.

5. Make title lookup forgiving without making it ambiguous. `notes_get` accepts an explicit `title`; when a caller places a title in `id`, Odo first attempts exact ID lookup and then falls back to a case-insensitive exact-title lookup. Duplicate titles still require disambiguation by ID.

6. Normalize unknown or empty `parentId` to a root folder only during creation. Updates continue to reject missing parents and circular relationships because silently changing a move would be destructive and surprising.

7. Return backup identifiers, not paths. `workspace_backup` returns the generated filename as `backupId`; the app keeps the full destination private in its configured backup directory.

8. Make template discovery visible from collection resource descriptions while retaining the standard `resources/templates/list` endpoint. Listing template URIs as concrete resources was rejected because clients could incorrectly attempt to read the literal braces.

## Risks / Trade-offs

- [Clients may have cached old schemas or prompt/resource metadata] → Restart the app and MCP client session, and verify the fresh live server after bundling.
- [Disabling the SSE retry hint removes a server-provided reconnect delay] → Clients retain normal SSE reconnection defaults; stateful session IDs and event IDs remain available.
- [Title fallback can match a note when an unknown ID equals another note's title] → Exact ID lookup always wins, and duplicate titles return an explicit ambiguity error.
- [A zero-sized page cannot communicate unseen matches] → It is intentionally terminal and documented; callers that need data must request a positive limit.

## Migration Plan

No database migration is required. Rebuild and restart Odo so clients receive new transport behavior and discovery metadata. Existing backups and workspace content remain unchanged.

## Open Questions

None for this change.
