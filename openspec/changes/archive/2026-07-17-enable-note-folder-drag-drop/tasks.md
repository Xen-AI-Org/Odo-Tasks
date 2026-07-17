## 1. Drag Interaction

- [x] 1.1 Restrict folder drop targets to in-process Odo note drags and keep mouse drops routed through the persisted note move operation
- [x] 1.2 Add touch and pen pointer dragging with threshold, destination hit testing, cancellation, and shared click suppression
- [x] 1.3 Add a visible note-row drag affordance and pointer drag preview/target styling

## 2. Verification

- [x] 2.1 Expand Playwright coverage for root and nested folder moves, persistence after reload, and rejection of external text drags
- [x] 2.2 Run the TypeScript production build and focused Playwright drag/drop tests
- [x] 2.3 Run Rust `cargo check` through the project validation command to confirm the unchanged native integration still compiles
