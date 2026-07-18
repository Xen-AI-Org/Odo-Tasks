# mcp-mutation-validation Specification

## Purpose
TBD - created by archiving change harden-mcp-mutation-validation. Update Purpose after archive.
## Requirements
### Requirement: Valid journal date keys
The `journal_upsert` tool SHALL accept only real calendar dates in exact `YYYY-MM-DD` form and SHALL reject malformed, blank, or impossible dates before storing an entry.

#### Scenario: Reject malformed journal dates
- **WHEN** a client calls `journal_upsert` with an empty value, a locale-formatted date, or an impossible date such as `2026-13-45`
- **THEN** the call returns a validation error and no journal entry is written

#### Scenario: Accept a leap-day journal date
- **WHEN** a client calls `journal_upsert` with a valid date such as `2028-02-29`
- **THEN** the call creates or updates the journal entry for that exact date

### Requirement: Valid task creation fields
The `tasks_create` tool SHALL reject blank task text, priorities outside `low`, `medium`, and `high`, scheduled starts that are not timezone-qualified RFC 3339-compatible ISO-8601 timestamps, and supplied colors that are not six-digit hexadecimal values in `#RRGGBB` form.

#### Scenario: Reject invalid task strings
- **WHEN** a client creates a task with whitespace-only text, an unknown priority, a malformed scheduled start, or a non-hex explicit color
- **THEN** the call returns a specific validation error and no task is written

#### Scenario: Accept a valid scheduled task
- **WHEN** a client creates a non-empty task with an allowed priority, a timezone-qualified ISO-8601 timestamp, and a six-digit hex color
- **THEN** the complete task is created with those supplied fields

### Requirement: Discoverable and bounded task numbers
Task effort SHALL advertise a minimum of 1 and maximum of 5 while retaining its runtime clamp, and task duration SHALL advertise a 1–1,440 minute range, reject non-positive values, and clamp larger positive values to 1,440 minutes.

#### Scenario: Inspect numeric constraints
- **WHEN** a client inspects the `tasks_create` input schema
- **THEN** effort declares minimum 1 and maximum 5 and duration declares minimum 1 and maximum 1,440

#### Scenario: Reject a non-positive duration
- **WHEN** a client calls `tasks_create` with a duration of zero or less
- **THEN** the call returns a validation error and no task is written

#### Scenario: Clamp an oversized duration
- **WHEN** a client calls `tasks_create` with a duration greater than 1,440 minutes
- **THEN** the created task reports and stores a duration of 1,440 minutes

### Requirement: Bounded and documented folder creation
Folder creation and rename SHALL reject blank names and names longer than 1,000 characters, and the `parentId` field schema SHALL document that missing, empty, or unknown values create a root-level folder with `parentId=null`.

#### Scenario: Reject an oversized folder name
- **WHEN** a client creates or renames a folder with a name longer than 1,000 characters
- **THEN** the call returns a validation error and stores no oversized name

#### Scenario: Discover and use parent fallback
- **WHEN** a client inspects `folders_create` and then supplies a nonexistent `parentId`
- **THEN** the schema explains the fallback and the returned folder has `parentId=null`

### Requirement: Complete category update response
The `categories_update` tool SHALL return the complete updated category object, including its ID, name, color, icon, and position.

#### Scenario: Verify category update directly
- **WHEN** a client renames or recolors an existing category
- **THEN** the mutation response contains every updated category field without requiring `categories_list`

### Requirement: Revision-safe batch note mutations
The `batch_apply` discovery description SHALL state that `notes.update`, `notes.move`, `notes.status`, and `notes.pin` require `expectedRevision`, and the runtime SHALL reject those operations when the revision is missing or stale.

#### Scenario: Reject an unprotected batch note mutation
- **WHEN** a client submits a protected batch note operation without `expectedRevision`
- **THEN** the entire batch is rejected without applying any operation

