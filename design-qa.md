# Odo Projects — Design QA

- Source visual truth: `/var/folders/zz/r1hpg4td5js_19hwjd6k0sf00000gn/T/TemporaryItems/NSIRD_screencaptureui_lZiTNq/Screenshot 2026-07-18 at 8.29.04 PM.png` and `/var/folders/zz/r1hpg4td5js_19hwjd6k0sf00000gn/T/TemporaryItems/NSIRD_screencaptureui_bcymhE/Screenshot 2026-07-18 at 8.29.59 PM.png`
- Implementation screenshots: `docs/qa/projects-board.jpg` and `docs/qa/project-detail.jpg`
- Side-by-side evidence: `docs/qa/projects-board-comparison.jpg` and `docs/qa/project-detail-comparison.jpg`
- Viewports: project board at 977 × 840; project detail at 986 × 919; responsive checks at 390 × 844
- States: all-project status board; Summer launch overview; Mobile capture task tab; empty related-task state

## Full-view comparison evidence

The side-by-side comparisons were opened at original resolution. The implementation preserves the source's core product structure: horizontally scrollable status columns, compact status headers, white project cards with descriptions and task counts, and a project detail page with breadcrumb navigation, tabs, a prominent title, properties, description, and open work. The implementation intentionally retains Odo's persistent sidebar and warm paper-like design system because this is an existing product surface rather than a standalone clone.

The source's milestone, activity, assignee, and resource surfaces were intentionally omitted. Related tasks and progress take their place, matching the requested scope.

## Focused region comparison evidence

Separate cropped comparisons were not needed. The original-resolution comparison images keep the board column headers, project cards, detail properties, description, and task area legible. There are no photography, logos, illustrations, or decorative raster assets to compare; all visible interface icons use the existing Phosphor icon family.

## Findings

No actionable P0, P1, or P2 differences remain.

- [P3] The project detail title uses Odo's editorial serif rather than the source's neutral sans serif.
  - Location: project detail hero.
  - Evidence: the source uses a compact system-sans title; the implementation uses the same Georgia title treatment as Odo's existing note, journal, and task surfaces.
  - Impact: minor visual deviation that improves consistency within the existing product.
  - Follow-up: switch only the project title to the system sans stack if stricter source fidelity is preferred over product consistency.

## Required fidelity surfaces

- Fonts and typography: existing Odo Georgia and system-sans stacks are used consistently. Card titles, summaries, status labels, metadata, truncation, and responsive wrapping were checked at the target desktop and mobile viewports.
- Spacing and layout rhythm: board column widths, compact headers, card padding, low radii, hairline borders, detail content width, property spacing, and vertical section rhythm closely match the references while fitting the persistent Odo navigation.
- Colors and visual tokens: the implementation stays within Odo's warm white, charcoal, muted gray, yellow, green, and clay tokens. Status colors remain semantic and readable without introducing a competing palette.
- Image quality and asset fidelity: the target contains no non-UI imagery. Phosphor icons are used throughout; no emoji, placeholder art, custom SVG, or CSS illustration substitutes were introduced.
- Copy and content: project names, summaries, descriptions, status labels, target dates, open-task counts, progress, and empty-state copy form a coherent standalone flow. Milestones and multi-assignee copy are absent as requested.
- Responsiveness and accessibility: desktop boards scroll horizontally without document overflow; the 390px layout shows one 300px board lane plus a clear next-lane cue. Project detail collapses to one column, controls wrap without clipping, focus indicators remain visible, and all tested controls expose accessible labels.

## Interaction verification

- Projects navigation opens the board.
- Selecting Summer launch opens its overview.
- Back navigation returns to the board.
- Overview and Tasks tabs switch correctly.
- Add task opens the project-scoped task dialog.
- Project status, target date, name, and description controls are wired to persisted state.
- Task details include a Project selector for linking and unlinking work.
- Desktop database schema stores projects and task-project relationships.
- Browser console errors checked: none.
- Frontend production build: passed.
- Rust library tests: 18 passed.

## Comparison history

1. Initial rendered comparison found no actionable P0/P1/P2 differences. The remaining title-font difference is intentional product-system alignment and is classified as P3.

## Implementation checklist

- [x] Add Projects to the main navigation.
- [x] Build the horizontal project status board and responsive lane behavior.
- [x] Open project cards into a dedicated overview and task view.
- [x] Support project creation, status, target date, name, description, and related tasks.
- [x] Persist projects and task links in browser and desktop storage.
- [x] Verify primary interactions, desktop/mobile layouts, console state, build, and Rust tests.

## Follow-up polish

- Optionally add drag-and-drop status changes between project columns in a later iteration.

final result: passed
