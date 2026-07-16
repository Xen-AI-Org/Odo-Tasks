## Why

Odo users need a direct, dependable way to reorganize notes without opening a move dialog. The existing note list exposes partial browser drag behavior, but the interaction is not specified as a product capability and does not cover robust pointer handling or complete destination validation.

## What Changes

- Let users drag an active, archived, or trashed note from the note list onto any visible folder, including Inbox and nested folders.
- Give clear source and destination feedback while a note is being dragged.
- Persist the note's new folder and active status, then open the destination folder with the moved note selected.
- Support pointer-driven dragging in addition to native HTML drag events so the interaction works consistently in Odo's desktop webview.
- Add end-to-end coverage for nested-folder moves, persistence after reload, and invalid/non-note drag payloads.
- Non-goals: this change does not add multi-note dragging, folder reordering, or dragging folders into other folders.

## Capabilities

### New Capabilities

- `note-folder-drag-drop`: Direct note movement from the note list to visible folders with feedback, persistence, and desktop-compatible input handling.

### Modified Capabilities

None.

## Impact

- Frontend note-row rendering and drag/drop event handling in `src/main.ts`.
- Drag source, preview, and folder target styles in `src/styles.css`.
- Playwright end-to-end tests under `tests/e2e`.
- No public API, storage schema, Rust backend, or new runtime dependency changes.
