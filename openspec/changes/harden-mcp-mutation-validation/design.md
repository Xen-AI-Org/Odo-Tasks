## Context

Odo generates MCP discovery schemas from Rust argument structs and persists mutation inputs directly to SQLite. Several arguments currently expose only primitive types and are not validated in handlers, while task effort and duration are silently normalized. The change spans generated schemas, runtime validation, response shaping, and protocol-level tests but does not change the database model.

## Goals / Non-Goals

**Goals:**

- Keep generated schemas and runtime behavior aligned for dates, task fields, folder names, and documented normalization.
- Reject malformed values before any write, change-counter bump, audit entry, or resource notification.
- Preserve existing defaults while making every non-default normalization explicit.
- Make category updates verifiable from their direct response.
- Cover discovery and mutation behavior through the real stdio MCP boundary.

**Non-Goals:**

- Cleaning up malformed rows that may already exist.
- Changing the planner's UI priority choices or duration controls.
- Changing folder creation's root fallback into an error.
- Removing optimistic revision checks from batch note mutations.

## Decisions

1. Use small shared validation helpers before acquiring mutation results. Calendar dates are parsed as strict `%Y-%m-%d`; scheduled timestamps are parsed as RFC 3339, the timezone-qualified profile used by the frontend's ISO-8601 output. A direct `chrono` dependency is preferred over a handwritten date/time parser because it correctly handles leap years, offsets, and component ranges.

2. Keep storage structs as strings and integers, but enrich their generated JSON Schemas with patterns, enums, ranges, formats, and field descriptions. Runtime checks remain authoritative because clients can bypass discovery constraints.

3. Validate task text after trimming only for emptiness and preserve the caller's original text. Explicit colors must use six-digit `#RRGGBB`; omitting color retains the existing empty-string default. Priority is limited to `low`, `medium`, or `high` as specified by the MCP contract.

4. Treat duration as a positive minute count with a one-day ceiling. Values at or below zero are rejected; values above 1,440 are stored as 1,440. The schema advertises the 1–1,440 range even though the runtime accepts oversized values in order to apply the documented clamp.

5. Apply the 1,000-character folder-name limit to both create and update. The same helper checks trimmed emptiness and Unicode scalar count so an update cannot introduce a value creation would reject.

6. Reuse the existing category lookup helper after a successful update. Returning the complete row avoids duplicating JSON response construction and keeps create/update shapes identical.

## Risks / Trade-offs

- [Existing clients send timezone-free local timestamps] → Reject them with a clear error and advertise `date-time`; the frontend already stores UTC ISO timestamps.
- [The UI currently knows an `urgent` priority] → Restrict only the MCP create contract requested here; existing persisted rows and frontend actions are not migrated.
- [Schema maximum suggests clients should not send oversized durations while runtime clamps them] → Document the clamp in the field and tool descriptions and test both schema metadata and direct runtime behavior.
- [Character counts differ from byte counts] → Count Unicode scalar values, matching the existing note-title policy.

## Migration Plan

No database migration is required. Rebuild and restart Odo so MCP clients receive the new discovery schema and handlers. Rollback consists of reverting the application build; existing valid rows remain compatible.

## Open Questions

None.
