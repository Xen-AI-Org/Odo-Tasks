## 1. Validation Contracts

- [x] 1.1 Add schema constraints and shared runtime validators for journal dates, task text, priority, effort, color, scheduled start, and duration
- [x] 1.2 Enforce 1–1,000 character folder names and document root fallback on the `parentId` field

## 2. Mutation Responses and Safety

- [x] 2.1 Return the complete updated category object from `categories_update`
- [x] 2.2 Verify `batch_apply` discovery and runtime retain `expectedRevision` requirements for protected note mutations

## 3. Regression Coverage and Validation

- [x] 3.1 Add focused Rust tests for the new validation helpers and boundary behavior
- [x] 3.2 Extend the stdio MCP conformance matrix to inspect schemas and exercise invalid and normalized mutations
- [x] 3.3 Run TypeScript checks/build, Rust format/tests/strict Clippy, MCP conformance, and OpenSpec validation
