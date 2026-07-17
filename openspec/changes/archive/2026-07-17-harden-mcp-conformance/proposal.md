## Why

Odo's MCP server must advertise the same capabilities it actually implements and behave predictably across strict clients. Several discovery, validation, pagination, privacy, and HTTP streaming edge cases currently produce stale-looking schemas, surprising results, or unnecessary local information disclosure.

## What Changes

- Make Streamable HTTP SSE output consumable by strict parsers without empty `data:` records.
- Keep backup responses privacy-safe by returning an opaque backup identifier instead of an absolute filesystem path.
- Fully document separator direction, content redaction, default note content omission, resource templates, prompts, and resource MIME types.
- Advertise and honor task text filtering, offset pagination, title-based note lookup, and complete task/folder update responses.
- Define deterministic zero-limit and empty-query behavior.
- Enforce non-empty note titles with a 1,000-character maximum.
- Normalize missing or empty folder parents to the root during folder creation while retaining cycle protection for updates.
- Add an end-to-end MCP protocol matrix that verifies schemas and behavior over real transports.

Non-goals: changing Odo's frontend navigation, exposing backup filesystem locations, or weakening revision and folder-cycle safety.

## Capabilities

### New Capabilities

- `mcp-conformance`: Discovery metadata, tool validation, pagination, privacy-safe responses, strict streaming behavior, and end-to-end MCP compatibility requirements.

### Modified Capabilities

None.

## Impact

- Rust MCP implementation and HTTP transport in `src-tauri/src/mcp.rs`.
- Workspace validation in `src-tauri/src/lib.rs` where shared constraints apply.
- MCP documentation and OpenSpec regression artifacts.
- No new runtime dependencies are expected.
