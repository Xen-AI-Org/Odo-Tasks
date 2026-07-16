## Context

Odo renders its notes and folder tree in one TypeScript module and persists the complete workspace through the existing `saveState` path. Native HTML drag events are already wired to note rows, but browser drag-and-drop is mouse-oriented, accepts overly broad text payloads, and is inconsistent for touch and pen input in embedded desktop webviews.

## Goals / Non-Goals

**Goals:**

- Make every rendered note row an obvious drag source and every visible folder an intentional destination.
- Use the same move-and-save operation for mouse, touch, pen, Inbox, and nested folders.
- Keep native mouse drag support while adding a pointer fallback for input types that do not emit HTML drag events.
- Preserve the existing move dialog as the keyboard-accessible alternative.
- Verify state changes and persistence through the browser-facing application boundary.

**Non-Goals:**

- Multi-note dragging or selection.
- Reordering or nesting folders.
- Replacing the workspace persistence format or the Rust storage commands.
- Changing note-to-note manual reordering behavior.

## Decisions

1. **Keep native HTML drag for mouse input and add a Pointer Events fallback for touch and pen.** Native drag supplies the most familiar desktop cursor behavior and integrates with the existing preview, while Pointer Events cover input devices for which HTML drag is absent. Replacing native drag for every device was considered, but it would duplicate browser behavior and make mouse automation less representative.

2. **Route all successful folder drops through `moveDroppedNote`.** The pointer fallback resolves the folder under the pointer and calls the existing mutation path. This keeps status restoration, ancestor expansion, destination selection, saving, and rendering identical across input methods.

3. **Recognize only Odo note drags.** Native targets accept the custom `application/x-odo-note` type or an in-process `draggingNoteId`; arbitrary `text/plain` drags are ignored. This prevents external text or URL drags from activating folder targets.

4. **Expose a non-interactive grip affordance inside each note row.** The whole row remains draggable, while the grip communicates the capability without creating another focus stop or interfering with row activation.

5. **Test behavior through Playwright and local storage.** Tests drag into both root and nested folders, assert destination UI state, inspect the persisted note record, reload, and verify the moved note remains in place. A synthetic external text drag verifies payload filtering.

## Risks / Trade-offs

- **[Risk] A pointer gesture may accidentally activate a note after dragging.** → Reuse the existing post-drag click suppression window and apply it to pointer fallback completion.
- **[Risk] Touch scrolling can conflict with dragging note rows.** → Start only after a movement threshold, prevent default only after the drag begins, and limit the fallback to non-mouse pointers.
- **[Risk] Rendering after a drop removes the source and target elements while handlers are completing.** → Capture the note and destination identifiers before clearing visual state or invoking the asynchronous move.
- **[Trade-off] Only visible folders can be targeted.** → Expandable folder navigation remains explicit; automatic hover expansion is deferred to avoid surprising tree changes.
