# note-folder-drag-drop Specification

## Purpose
TBD - created by archiving change enable-note-folder-drag-drop. Update Purpose after archive.
## Requirements
### Requirement: Move a note directly to a folder
Odo SHALL allow a user to drag any note shown in the note list onto any visible folder target, including Inbox and nested folders.

#### Scenario: Move an active note to a root folder
- **WHEN** the user drags an active note onto a different root folder
- **THEN** Odo assigns the note to that folder, keeps it active, opens the destination view, and selects the moved note

#### Scenario: Move a note to a nested folder
- **WHEN** the user drags a note onto a visible nested folder
- **THEN** Odo assigns the note to the exact nested folder and leaves its ancestor path expanded

#### Scenario: Restore a non-active note to a folder
- **WHEN** the user drags an archived or trashed note onto a folder
- **THEN** Odo changes the note status to active and moves it into the destination folder

### Requirement: Persist a folder drop
Odo SHALL save a successful folder drop through the existing workspace persistence path before presenting the move as complete.

#### Scenario: Reload after moving a note
- **WHEN** a note has been dropped into a different folder and the application is reloaded
- **THEN** the note remains assigned to the destination folder and is visible there

### Requirement: Communicate and validate drag state
Odo SHALL show distinct source and destination feedback during note dragging and SHALL activate folder targets only for an in-process Odo note drag.

#### Scenario: Drag over a valid folder
- **WHEN** an Odo note drag enters a folder target
- **THEN** the note source and folder destination display drag feedback and the live region identifies the destination

#### Scenario: Drag external text over a folder
- **WHEN** text or a URL from outside the note list is dragged over a folder
- **THEN** the folder does not activate and no note state changes

### Requirement: Support desktop pointer input
Odo SHALL support folder dragging with mouse input and SHALL provide equivalent movement for touch or pen pointer input.

#### Scenario: Move with touch or pen
- **WHEN** the user moves a touch or pen pointer beyond the drag threshold and releases it over a folder
- **THEN** Odo moves the note through the same validated and persisted operation used for a mouse drop

#### Scenario: Release outside a folder
- **WHEN** a pointer drag ends without a valid folder under the pointer
- **THEN** Odo cancels the move, clears drag feedback, and leaves the note unchanged

