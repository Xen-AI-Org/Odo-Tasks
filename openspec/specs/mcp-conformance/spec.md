# mcp-conformance Specification

## Purpose
TBD - created by archiving change harden-mcp-conformance. Update Purpose after archive.
## Requirements
### Requirement: Strict Streamable HTTP events
The Streamable HTTP server SHALL emit real JSON-RPC SSE data records without an empty `data:` priming record.

#### Scenario: Initialize over HTTP
- **WHEN** a client sends an MCP initialize request accepting `text/event-stream`
- **THEN** every emitted `data:` record contains a non-empty JSON-RPC payload

### Requirement: Complete discovery metadata
The server SHALL describe every advertised prompt and resource, SHALL advertise resource MIME types that match reads, and SHALL mention the note and journal template URIs in collection resource discovery while retaining `resources/templates/list`.

#### Scenario: Client discovers resources and prompts
- **WHEN** a client calls `resources/list`, `resources/templates/list`, and `prompts/list`
- **THEN** resources have descriptions and `text/markdown` MIME types, collection descriptions mention their template URIs, both templates are returned, and all five prompts have descriptions

### Requirement: Predictable pagination and filtering
The `notes_list`, `tasks_list`, `search`, and `activity_list` tools SHALL advertise and honor `offset`; `tasks_list` SHALL advertise and honor `query`; note content omission SHALL be documented; and `limit=0` SHALL return an empty terminal page.

#### Scenario: Client requests a later page
- **WHEN** a client passes a positive limit and an advertised offset
- **THEN** the tool returns the requested page with matching `limit`, `offset`, `hasMore`, and `nextOffset` metadata

#### Scenario: Client requests zero items
- **WHEN** a client passes `limit=0`
- **THEN** the tool returns zero items with `hasMore=false` and no next offset

#### Scenario: Client filters tasks by text
- **WHEN** a client passes `query` to `tasks_list`
- **THEN** only tasks whose text contains that query are returned

### Requirement: Explicit search validation
The global `search` tool SHALL reject an empty or whitespace-only query with `Query must not be empty`.

#### Scenario: Empty global search
- **WHEN** a client calls `search` with an empty query
- **THEN** the call returns a structured tool error and does not return workspace content

### Requirement: Safe and discoverable note operations
Note creation and title updates SHALL reject blank titles and titles longer than 1,000 characters, `notes_get` SHALL support exact case-insensitive title lookup, and append/prepend descriptions SHALL state where separators are inserted.

#### Scenario: Invalid note title
- **WHEN** a client creates or updates a note with a blank title or a title longer than 1,000 characters
- **THEN** Odo rejects the operation with a specific validation error and stores no invalid title

#### Scenario: Lookup by title
- **WHEN** a client calls `notes_get` using a unique exact title in either the `title` field or as a fallback value in `id`
- **THEN** Odo returns the complete matching note

#### Scenario: Separator direction discovery
- **WHEN** a client inspects `notes_append` and `notes_prepend`
- **THEN** the descriptions state that append places the separator before new text and prepend places it after new text

### Requirement: Privacy-safe operational responses
`workspace_backup` SHALL return an opaque backup identifier without an absolute path, and `activity_list` SHALL state that content fields are redacted for privacy.

#### Scenario: Workspace backup succeeds
- **WHEN** a client calls `workspace_backup`
- **THEN** the response identifies the backup without revealing the user's home or application-support path

#### Scenario: Client inspects activity audit
- **WHEN** a client inspects the `activity_list` description and results
- **THEN** the description explains content redaction and results contain redacted details instead of note or journal bodies

### Requirement: Robust folder and mutation behavior
Folder creation SHALL normalize an empty or missing parent reference to the root, folder updates SHALL remain cycle-safe, and task and folder update tools SHALL return complete updated objects.

#### Scenario: Create folder with unknown parent
- **WHEN** a client calls `folders_create` with an empty or nonexistent `parentId`
- **THEN** Odo creates the folder at the root with `parentId=null`

#### Scenario: Verify a mutation without another read
- **WHEN** a client updates a task or folder
- **THEN** the response contains the complete updated object

#### Scenario: Reject circular folder update
- **WHEN** a client attempts to make a folder a descendant of itself
- **THEN** Odo rejects the update without leaving a SQLite write transaction open

