## Why

Several MCP mutation tools still accept malformed dates, task fields, and oversized names, allowing junk data to enter the workspace and making discovery schemas weaker than runtime behavior. The remaining response-shape inconsistency also forces clients to perform an extra category read after updates.

## What Changes

- Validate journal date keys as real `YYYY-MM-DD` calendar dates before upserting entries.
- Reject blank task text, unknown priorities, malformed scheduled timestamps, and malformed explicit task colors.
- Advertise the task effort range and enforce a bounded duration policy: reject non-positive durations and clamp durations above one day.
- Limit folder names to 1,000 characters on creation and update, and document that missing or unknown creation parents become root folders.
- Return the complete updated category object from `categories_update`.
- Retain and regression-test revision protection for note mutations in `batch_apply`.

Non-goals: migrating existing workspace rows, changing frontend planner controls, rejecting unknown folder parents during creation, or weakening note revision checks.

## Capabilities

### New Capabilities

- `mcp-mutation-validation`: Input validation, schema constraints, response completeness, and mutation-safety requirements for MCP journal, task, folder, category, and batch tools.

### Modified Capabilities

None.

## Impact

- MCP argument schemas, validation helpers, and mutation handlers in `src-tauri/src/mcp.rs`.
- Rust unit tests and the stdio MCP conformance matrix.
- OpenSpec contract and implementation tasks.
- No database migration or new persisted fields.
