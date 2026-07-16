## 1. Transport and Privacy

- [x] 1.1 Disable the empty rmcp SSE priming record while preserving stateful HTTP sessions and real JSON-RPC events
- [x] 1.2 Replace the absolute backup path response with an opaque backup identifier

## 2. Tool and Discovery Contracts

- [x] 2.1 Complete separator, redaction, note-content, resource-template, resource MIME, and prompt descriptions
- [x] 2.2 Verify task text filtering and offset schemas, and implement deterministic zero-limit and empty-search behavior
- [x] 2.3 Enforce 1-1,000 character note titles and make title lookup work through both explicit title and ID fallback inputs
- [x] 2.4 Normalize missing folder parents to root on creation and return complete task/folder update objects

## 3. Regression Coverage and Validation

- [x] 3.1 Add focused Rust tests for title validation, zero-limit metadata, parent fallback, and privacy-safe backup identifiers
- [x] 3.2 Add a repeatable stdio MCP protocol matrix that inspects discovery schemas and exercises every reported behavior
- [x] 3.3 Rebuild and verify live Streamable HTTP output contains no empty SSE data records
- [x] 3.4 Run TypeScript build/check, Rust tests, strict Clippy, OpenSpec validation, and macOS app bundling
