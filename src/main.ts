import "@phosphor-icons/web/regular/style.css";
import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

type NoteStatus = "active" | "archived" | "trash";
type View = "notes" | "tasks" | "settings" | "archived" | "trash";
type Folder = { id: string; name: string; parentId: string | null; open: boolean; icon?: string };
type Note = { id: string; folderId: string; title: string; content: string; updated: string; status: NoteStatus; pinned?: boolean };
type Todo = { id: string; text: string; completed: boolean; created: string; updated: string };
type Workspace = { folders: Folder[]; notes: Note[]; todos: Todo[]; selectedFolderId: string; selectedNoteId: string };
type StorageInfo = { databasePath: string; backupDirectory: string };
type SavePhase = "idle" | "saving" | "saved" | "error";
type MenuItem = { label?: string; icon?: string; hint?: string; action?: string; disabled?: boolean; danger?: boolean; separator?: boolean };
type MenuState = { items: MenuItem[]; x: number; y: number; trigger: HTMLElement | null } | null;

const STORAGE_KEY = "odo-notes-workspace-v2";
const MOTION_KEY = "odo-motion-enabled";
const isDesktopApp = "__TAURI_INTERNALS__" in window;
const isMac = navigator.platform.toLowerCase().includes("mac");
const modLabel = isMac ? "Cmd" : "Ctrl";
const uid = (prefix: string) => `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
const now = () => new Date().toISOString();

const starterFolders: Folder[] = [
  { id: "inbox", name: "Inbox", parentId: null, open: true, icon: "ph-tray" },
  { id: "work", name: "Work", parentId: null, open: true, icon: "ph-briefcase" },
  { id: "projects", name: "Projects", parentId: "work", open: true },
  { id: "summer", name: "Summer launch", parentId: "projects", open: true },
  { id: "personal", name: "Personal", parentId: null, open: true, icon: "ph-user-circle" },
  { id: "journal", name: "Journal", parentId: "personal", open: false },
];
const starterNotes: Note[] = [
  { id: "welcome", folderId: "inbox", title: "Welcome to Odo", updated: now(), status: "active", pinned: true, content: "## A calm place for your work\n\nCapture ideas, organize projects, and keep your day moving.\n\n- Press Ctrl+N for a new note\n- Press Ctrl+2 for Tasks\n- Type / on a new line for blocks" },
];
const starterWorkspace = (): Workspace => ({ folders: structuredClone(starterFolders), notes: structuredClone(starterNotes), todos: [], selectedFolderId: "inbox", selectedNoteId: "welcome" });

const escapeHtml = (value: string) => value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/\"/g, "&quot;").replace(/'/g, "&#039;");
const attr = escapeHtml;
function normalizeWorkspace(input: Partial<Workspace> | null | undefined): Workspace {
  const fallback = starterWorkspace();
  const folders = Array.isArray(input?.folders) ? input.folders : fallback.folders;
  if (!folders.some((folder) => folder.id === "inbox")) folders.unshift({ id: "inbox", name: "Inbox", parentId: null, open: true, icon: "ph-tray" });
  return { folders, notes: Array.isArray(input?.notes) ? input.notes : fallback.notes, todos: Array.isArray(input?.todos) ? input.todos : [], selectedFolderId: input?.selectedFolderId || "inbox", selectedNoteId: input?.selectedNoteId || "" };
}
function initialState(): Workspace {
  try { const saved = localStorage.getItem(STORAGE_KEY); if (saved) return normalizeWorkspace(JSON.parse(saved) as Workspace); } catch { localStorage.removeItem(STORAGE_KEY); }
  return starterWorkspace();
}

let state = initialState();
let currentView: View = "notes";
let searchQuery = "";
let sortNewest = true;
let focusMode = false;
let sidebarCollapsed = false;
let slashOpen = false;
let slashIndex = 0;
let saveTimer = 0;
let savePhase: SavePhase = "idle";
let saveSequence = 0;
let desktopSaveChain: Promise<void> = Promise.resolve();
let menuState: MenuState = null;
let completedCollapsed = false;
let editingTodoId = "";
let helpOpen = false;
let storageInfo: StorageInfo | null = null;
let storageError = "";
let motionEnabled = localStorage.getItem(MOTION_KEY) !== "false";

const commands = [
  { label: "Text", detail: "Just start writing.", icon: "ph-text-t", value: "" },
  { label: "Heading", detail: "Add a heading.", icon: "ph-text-h", value: "## " },
  { label: "To-do list", detail: "Track tasks in this note.", icon: "ph-check-square", value: "- [ ] " },
  { label: "Bulleted list", detail: "Create a simple list.", icon: "ph-list-bullets", value: "- " },
  { label: "Callout", detail: "Make something stand out.", icon: "ph-chat-centered-text", value: "> " },
  { label: "Code", detail: "Write a code block.", icon: "ph-code", value: "```\n\n```" },
];

function descendants(id: string, seen = new Set<string>()): string[] {
  if (seen.has(id)) return [];
  seen.add(id);
  return [id, ...state.folders.filter((folder) => folder.parentId === id).flatMap((folder) => descendants(folder.id, seen))];
}
function currentFolder(): Folder { return state.folders.find((folder) => folder.id === state.selectedFolderId) ?? state.folders.find((folder) => folder.id === "inbox")!; }
function selectedNote(): Note | undefined { return state.notes.find((note) => note.id === state.selectedNoteId); }
function statusForView(): NoteStatus { return currentView === "archived" ? "archived" : currentView === "trash" ? "trash" : "active"; }
function visibleNotes(): Note[] {
  const folderIds = currentView === "notes" ? descendants(state.selectedFolderId) : null;
  return state.notes.filter((note) => note.status === statusForView()).filter((note) => !folderIds || folderIds.includes(note.folderId)).filter((note) => `${note.title} ${note.content}`.toLowerCase().includes(searchQuery.toLowerCase())).sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    const delta = new Date(b.updated).getTime() - new Date(a.updated).getTime();
    return sortNewest ? delta : -delta;
  });
}
function repairState() {
  if (!state.folders.some((folder) => folder.id === state.selectedFolderId)) state.selectedFolderId = "inbox";
  const note = selectedNote();
  if (!note || (currentView === "notes" && (note.status !== "active" || !descendants(state.selectedFolderId).includes(note.folderId))) || (currentView === "archived" && note.status !== "archived") || (currentView === "trash" && note.status !== "trash")) state.selectedNoteId = visibleNotes()[0]?.id ?? "";
}
function folderCount(id: string) { const ids = descendants(id); return state.notes.filter((note) => ids.includes(note.folderId) && note.status === "active").length; }
function viewTitle() { return currentView === "archived" ? "Archive" : currentView === "trash" ? "Trash" : currentFolder().name; }
function noteExcerpt(content: string) { return content.replace(/[#>*`|\[\]-]/g, " ").replace(/\s+/g, " ").trim().slice(0, 82) || "Empty note"; }
function formatListDate(iso: string) {
  const date = new Date(iso); const today = new Date(); const yesterday = new Date(); yesterday.setDate(today.getDate() - 1);
  if (date.toDateString() === today.toDateString()) return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (date.toDateString() === yesterday.toDateString()) return "Yesterday";
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}
function formatEditorDate(iso: string) { return new Date(iso).toLocaleString([], { month: "long", day: "numeric", year: "numeric", hour: "numeric", minute: "2-digit" }); }
function wordCount(content: string) { return content.trim() ? content.trim().split(/\s+/).length : 0; }

function setSavePhase(phase: SavePhase) { savePhase = phase; updateSaveStatus(); }
function updateSaveStatus() {
  const element = document.querySelector<HTMLElement>("#save-status"); if (!element) return;
  const content: Record<SavePhase, string> = { idle: "", saving: '<i class="ph ph-circle-notch"></i> Saving', saved: '<i class="ph ph-check-circle"></i> Saved', error: '<i class="ph ph-warning-circle"></i> Save failed' };
  element.className = `save-status ${savePhase}`; element.innerHTML = content[savePhase];
}
function saveState(show = true): Promise<void> {
  repairState();
  const snapshot = JSON.stringify(state); const sequence = ++saveSequence;
  if (show) setSavePhase("saving");
  if (!isDesktopApp) {
    try { localStorage.setItem(STORAGE_KEY, snapshot); if (show) setSavePhase("saved"); return Promise.resolve(); }
    catch (error) { console.error(error); if (show) setSavePhase("error"); return Promise.reject(error); }
  }
  desktopSaveChain = desktopSaveChain.catch(() => undefined).then(async () => {
    try { await invoke("save_workspace", { contents: snapshot }); if (show && sequence === saveSequence) setSavePhase("saved"); }
    catch (error) { console.error("Could not save Odo workspace:", error); if (sequence === saveSequence) setSavePhase("error"); }
  });
  return desktopSaveChain;
}
function scheduleSave() { clearTimeout(saveTimer); setSavePhase("saving"); saveTimer = window.setTimeout(() => void saveState(), 350); }
async function restoreDesktopWorkspace() {
  if (!isDesktopApp) return;
  try {
    const workspace = await invoke<string | null>("load_workspace");
    if (workspace) state = normalizeWorkspace(JSON.parse(workspace) as Workspace); else await saveState(false);
  } catch (error) { console.error("Could not restore Odo workspace:", error); setSavePhase("error"); }
  repairState(); renderApp();
}

function renderFolder(folder: Folder, depth = 0): string {
  const children = state.folders.filter((candidate) => candidate.parentId === folder.id);
  const toggle = children.length ? `<button class="folder-toggle" data-toggle-folder="${attr(folder.id)}" aria-label="${folder.open ? "Collapse" : "Expand"} ${attr(folder.name)}"><i class="ph ph-caret-${folder.open ? "down" : "right"}"></i></button>` : '<span class="folder-toggle-spacer"></span>';
  return `<div class="folder-branch"><div class="folder-row ${currentView === "notes" && state.selectedFolderId === folder.id ? "is-selected" : ""}" data-folder-id="${attr(folder.id)}" style="--depth:${depth}" role="button" tabindex="0" aria-label="${attr(folder.name)}, ${folderCount(folder.id)} notes">${toggle}<i class="ph ${folder.icon ?? "ph-folder"} folder-icon"></i><span class="folder-name">${escapeHtml(folder.name)}</span><span class="folder-count">${folderCount(folder.id)}</span><button class="row-more" data-folder-menu="${attr(folder.id)}" aria-label="Folder actions"><i class="ph ph-dots-three"></i></button></div>${folder.open ? children.map((child) => renderFolder(child, depth + 1)).join("") : ""}</div>`;
}
function renderSidebar() {
  const remaining = state.todos.filter((todo) => !todo.completed).length;
  return `<aside class="folders-panel" aria-label="Workspace navigation"><header class="brand-row"><button class="wordmark" id="wordmark" title="Go to Inbox">Odo</button><button class="icon-button sidebar-toggle" title="${sidebarCollapsed ? "Show" : "Hide"} sidebar" aria-label="${sidebarCollapsed ? "Show" : "Hide"} sidebar"><i class="ph ph-sidebar-simple"></i></button></header>
    <nav class="primary-nav"><button class="primary-link ${currentView === "notes" && state.selectedFolderId === "inbox" ? "is-selected" : ""}" data-go-inbox><i class="ph ph-tray"></i><span>Inbox</span><kbd>${modLabel}+1</kbd></button><button class="primary-link ${currentView === "tasks" ? "is-selected" : ""}" data-view="tasks"><i class="ph ph-check-square"></i><span>Tasks</span><span class="nav-count ${remaining ? "has-items" : ""}">${remaining}</span></button></nav>
    <div class="panel-label-row"><span>Folders</span><button class="icon-button" id="new-folder" title="New folder (${modLabel}+Shift+N)"><i class="ph ph-plus"></i></button></div><nav class="folder-tree" id="folder-tree">${state.folders.filter((folder) => folder.parentId === null && folder.id !== "inbox").map((folder) => renderFolder(folder)).join("")}</nav>
    <div class="library-links"><button class="library-link ${currentView === "archived" ? "is-selected" : ""}" data-view="archived"><i class="ph ph-archive-tray"></i><span>Archive</span><span>${state.notes.filter((note) => note.status === "archived").length}</span></button><button class="library-link ${currentView === "trash" ? "is-selected" : ""}" data-view="trash"><i class="ph ph-trash"></i><span>Trash</span><span>${state.notes.filter((note) => note.status === "trash").length}</span></button></div><button class="settings-link ${currentView === "settings" ? "is-selected" : ""}" data-view="settings"><i class="ph ph-gear"></i><span>Settings</span></button></aside>`;
}
function renderNotesPanel() {
  return `<section class="notes-panel"><div class="global-bar"><label class="search-box"><i class="ph ph-magnifying-glass"></i><input id="search-input" type="search" placeholder="Search notes..." value="${attr(searchQuery)}"><kbd>${modLabel}+K</kbd></label><button class="new-note-button" id="new-note" title="New note (${modLabel}+N)"><i class="ph ph-note-pencil"></i></button></div><header class="notes-header"><div><strong id="note-list-title">${escapeHtml(viewTitle())}</strong><span id="note-count">${visibleNotes().length} notes</span></div><button class="sort-button" id="list-menu" title="List actions" aria-label="List actions"><i class="ph ph-dots-three"></i></button></header><div class="note-list" id="note-list" tabindex="-1">${renderNoteRows()}</div></section>`;
}
function renderNoteRows() {
  const notes = visibleNotes();
  if (!notes.length) return `<div class="empty-state"><span class="empty-icon"><i class="ph ${currentView === "trash" ? "ph-trash" : currentView === "archived" ? "ph-archive-tray" : "ph-note-blank"}"></i></span><strong>${searchQuery ? "No matching notes" : currentView === "trash" ? "Trash is empty" : currentView === "archived" ? "Nothing archived" : "A clear page awaits"}</strong><p>${searchQuery ? "Try a different search." : "Capture a thought and give it somewhere to grow."}</p>${currentView === "notes" && !searchQuery ? '<button class="primary-button" data-create-note>Create a note</button>' : ""}</div>`;
  return notes.map((note) => `<div class="note-row ${note.id === state.selectedNoteId ? "is-selected" : ""}" data-note-id="${attr(note.id)}" role="button" tabindex="${note.id === state.selectedNoteId ? "0" : "-1"}"><div class="note-heading"><span class="note-title-wrap">${note.pinned ? '<i class="ph ph-push-pin pin-icon"></i>' : ""}<span class="note-title">${escapeHtml(note.title || "Untitled")}</span></span><time>${formatListDate(note.updated)}</time><button class="row-more" data-note-menu="${attr(note.id)}" aria-label="Note actions"><i class="ph ph-dots-three"></i></button></div><span class="note-excerpt">${escapeHtml(noteExcerpt(note.content))}</span></div>`).join("");
}
function slashMenuHtml() { return `<div class="slash-menu ${slashOpen ? "is-open" : ""}" id="slash-menu" role="listbox">${commands.map((command, index) => `<button class="slash-command ${index === slashIndex ? "is-active" : ""}" data-command-index="${index}" role="option" aria-selected="${index === slashIndex}"><span class="command-icon"><i class="ph ${command.icon}"></i></span><span><strong>${command.label}</strong><small>${command.detail}</small></span>${index === 0 ? "<kbd>Enter</kbd>" : ""}</button>`).join("")}</div>`; }
function renderEditor() {
  const note = selectedNote();
  if (!note) return `<main class="editor-panel empty-editor"><div class="app-actions"><span id="save-status"></span></div><div class="editor-empty"><span class="empty-icon"><i class="ph ph-note-pencil"></i></span><h2>No note selected</h2><p>Select a note, or start with a fresh page.</p><button class="primary-button" data-create-note>New note <kbd>${modLabel}+N</kbd></button></div></main>`;
  const canEdit = note.status === "active";
  return `<main class="editor-panel"><div class="app-actions"><span id="save-status" class="save-status ${savePhase}"></span><button class="action-button" id="focus-mode"><i class="ph ph-book-open-text"></i><span>${focusMode ? "Exit focus" : "Focus"}</span></button><span class="action-separator"></span>${note.status === "active" ? '<button class="icon-button" data-note-action="archive" title="Archive note"><i class="ph ph-archive-tray"></i></button>' : ""}<button class="icon-button" id="editor-menu" title="More note actions" aria-label="More note actions"><i class="ph ph-dots-three"></i></button></div>
    ${canEdit ? `<div class="format-bar" role="toolbar" aria-label="Formatting"><button data-insert="## ">H₁</button><button data-insert="### ">H₂</button><button data-insert="#### ">H₃</button><span></span><button data-wrap="**">B</button><button data-wrap="_" class="italic">I</button><button data-wrap="~~" class="strike">S</button><span></span><button class="icon-button" data-insert="- " title="Bulleted list"><i class="ph ph-list-bullets"></i></button><button class="icon-button" data-insert="- [ ] " title="To-do list"><i class="ph ph-check-square"></i></button><span></span><button class="icon-button" data-wrap="[]()" data-link title="Add link"><i class="ph ph-link"></i></button><button class="icon-button" data-wrap="\`" title="Inline code"><i class="ph ph-code"></i></button><span></span><button class="icon-button" id="toolbar-more" title="Block menu"><i class="ph ph-dots-three"></i></button><button class="icon-button expand-editor" id="expand-editor" title="Focus mode"><i class="ph ph-arrows-out"></i></button></div>` : '<div class="readonly-bar"><i class="ph ph-info"></i>This note is read-only here. Restore it to edit.</div>'}
    <article class="editor-page"><input class="title-input" id="title-input" value="${attr(note.title)}" aria-label="Note title" ${canEdit ? "" : "readonly"}><div class="note-meta"><span>${formatEditorDate(note.updated)}</span><span>·</span><span id="word-count">${wordCount(note.content)} words</span></div><div class="editor-wrap"><textarea id="markdown-editor" aria-label="Markdown content" spellcheck="true" ${canEdit ? "" : "readonly"}>${escapeHtml(note.content)}</textarea>${canEdit ? slashMenuHtml() : ""}</div></article></main>`;
}
function renderTasks() {
  const active = state.todos.filter((todo) => !todo.completed); const completed = state.todos.filter((todo) => todo.completed);
  return `<main class="wide-view tasks-view"><header class="wide-header"><div><span class="eyebrow">Daily workspace</span><h1>Tasks</h1><p>${active.length ? `${active.length} ${active.length === 1 ? "task" : "tasks"} left to move forward.` : "Everything is clear."}</p></div><div class="header-mark"><i class="ph ph-check-square"></i></div></header><section class="task-sheet"><form id="quick-task-form" class="quick-task"><i class="ph ph-plus"></i><input id="quick-task-input" autocomplete="off" placeholder="Add a task and press Enter" aria-label="New task"><kbd>Enter</kbd></form>
    <div class="task-section"><div class="task-section-heading"><h2>Up next</h2><span>${active.length}</span></div><div class="task-list" id="active-task-list" role="list">${active.length ? active.map(renderTodo).join("") : '<div class="task-empty"><i class="ph ph-sparkle"></i><strong>No active tasks</strong><span>Add one above, or enjoy the clear desk.</span></div>'}</div></div>
    <div class="task-section completed-section"><div class="task-section-heading"><button id="toggle-completed" aria-expanded="${!completedCollapsed}"><i class="ph ph-caret-${completedCollapsed ? "right" : "down"}"></i><h2>Completed</h2><span>${completed.length}</span></button>${completed.length ? '<button class="text-danger" id="clear-completed">Clear completed</button>' : ""}</div>${completedCollapsed ? "" : `<div class="task-list" id="completed-task-list" role="list">${completed.length ? completed.map(renderTodo).join("") : '<div class="completed-empty">Finished tasks will rest here.</div>'}</div>`}</div><footer class="task-hints"><span><kbd>↑</kbd><kbd>↓</kbd> Navigate</span><span><kbd>Space</kbd> Complete</span><span><kbd>Enter</kbd> Edit</span></footer></section></main>`;
}
function renderTodo(todo: Todo) {
  const editing = editingTodoId === todo.id;
  return `<div class="task-row ${todo.completed ? "is-complete" : ""} ${editing ? "is-editing" : ""}" data-todo-id="${attr(todo.id)}" role="listitem" tabindex="0"><button class="task-check" data-toggle-todo="${attr(todo.id)}" aria-label="${todo.completed ? "Reopen" : "Complete"} ${attr(todo.text)}"><i class="ph ph-check"></i></button>${editing ? `<input class="task-edit-input" data-edit-todo="${attr(todo.id)}" value="${attr(todo.text)}" aria-label="Edit task">` : `<span class="task-text">${escapeHtml(todo.text)}</span>`}<time>${formatListDate(todo.updated)}</time><button class="row-more task-more" data-todo-menu="${attr(todo.id)}" aria-label="Task actions"><i class="ph ph-dots-three"></i></button></div>`;
}
function renderSettings() {
  return `<main class="wide-view settings-view"><header class="wide-header"><div><span class="eyebrow">Preferences</span><h1>Settings</h1><p>Make Odo feel at home on this computer.</p></div><button class="icon-button close-wide" data-go-inbox title="Back to notes"><i class="ph ph-x"></i></button></header><div class="settings-sheet">
    <section class="settings-section"><div class="settings-copy"><i class="ph ph-database"></i><div><h2>Storage & backups</h2><p>${isDesktopApp ? "Your workspace is stored locally on this computer." : "Browser mode stores this workspace in local storage."}</p></div></div>${isDesktopApp ? `<div class="path-grid"><span>Database</span><code>${escapeHtml(storageInfo?.databasePath ?? "Loading…")}</code><span>Backups</span><code>${escapeHtml(storageInfo?.backupDirectory ?? "Loading…")}</code></div><button class="secondary-button" id="create-backup" ${storageError ? "disabled" : ""}><i class="ph ph-cloud-arrow-up"></i>Create backup</button>${storageError ? `<p class="inline-error">${escapeHtml(storageError)}</p>` : ""}` : '<div class="browser-note"><i class="ph ph-info"></i>Install and open the desktop app to create file backups.</div>'}</section>
    <section class="settings-section"><div class="settings-copy"><i class="ph ph-sparkle"></i><div><h2>Appearance & motion</h2><p>Keep transitions calm, quick, and comfortable.</p></div></div><label class="switch-row"><span>Interface motion<small>Menus, panels, and task feedback</small></span><input id="motion-toggle" type="checkbox" ${motionEnabled ? "checked" : ""}><span class="switch" aria-hidden="true"></span></label></section>
    <section class="settings-section"><div class="settings-copy"><i class="ph ph-keyboard"></i><div><h2>Keyboard shortcuts</h2><p>Everything important stays within reach.</p></div></div><div class="shortcut-grid">${[["New note",`${modLabel}+N`],["New folder",`${modLabel}+Shift+N`],["Search",`${modLabel}+K`],["Tasks",`${modLabel}+2`],["Save",`${modLabel}+S`],["Focus mode",`${modLabel}+Shift+F`]].map(([label, key]) => `<span>${label}</span><kbd>${key}</kbd>`).join("")}</div><button class="secondary-button" id="show-help"><i class="ph ph-question"></i>View all shortcuts</button></section></div></main>`;
}
function renderDialogLayer() {
  return `<dialog id="folder-dialog" class="create-dialog"><form id="folder-form"><div class="dialog-icon"><i class="ph ph-folder-plus"></i></div><div><h2>Create a new folder</h2><p>Add it inside the current location.</p></div><label>Folder name<input id="folder-name" autocomplete="off" placeholder="e.g. Research" required></label><div class="dialog-actions"><button type="button" class="secondary-button" data-close-dialog>Cancel</button><button type="submit" class="primary-button">Create</button></div></form></dialog>
    <dialog id="move-dialog" class="create-dialog"><form id="move-form"><div class="dialog-icon"><i class="ph ph-folder-notch-open"></i></div><div><h2>Move note</h2><p>Choose its new home.</p></div><label>Folder<select id="move-folder">${state.folders.map((folder) => `<option value="${attr(folder.id)}">${escapeHtml(folder.name)}</option>`).join("")}</select></label><div class="dialog-actions"><button type="button" class="secondary-button" data-close-dialog>Cancel</button><button type="submit" class="primary-button">Move</button></div></form></dialog>`;
}
function renderHelp() {
  if (!helpOpen) return "";
  const shortcuts = [[`${modLabel}+N`,"New note"],[`${modLabel}+Shift+N`,"New folder"],[`${modLabel}+K`,"Search"],[`${modLabel}+S`,"Save"],[`${modLabel}+1`,"Inbox"],[`${modLabel}+2`,"Tasks"],[`${modLabel}+Shift+T`,"Tasks"],[`${modLabel}+D`,"Duplicate note"],[`${modLabel}+E`,"Archive note"],["F2","Rename selection"],["?","Shortcut help"],["Esc","Close / exit focus"]];
  return `<div class="overlay" id="help-overlay"><section class="help-panel" role="dialog" aria-modal="true" aria-labelledby="help-title"><header><div><span class="eyebrow">Keyboard first</span><h2 id="help-title">Shortcuts</h2></div><button class="icon-button" id="close-help" aria-label="Close shortcuts"><i class="ph ph-x"></i></button></header><div class="help-grid">${shortcuts.map(([key,label]) => `<span>${label}</span><kbd>${key}</kbd>`).join("")}</div><p>Menus also support arrow keys, Home, End, Enter, and Escape.</p></section></div>`;
}
function renderMenu() {
  if (!menuState) return "";
  return `<div class="context-menu" id="context-menu" role="menu" style="left:${menuState.x}px;top:${menuState.y}px">${menuState.items.map((item, index) => item.separator ? '<div class="menu-separator" role="separator"></div>' : `<button role="menuitem" data-menu-index="${index}" ${item.disabled ? "disabled" : ""} class="${item.danger ? "danger" : ""}"><i class="ph ${item.icon ?? "ph-dot"}"></i><span>${escapeHtml(item.label ?? "")}</span>${item.hint ? `<kbd>${escapeHtml(item.hint)}</kbd>` : ""}</button>`).join("")}</div>`;
}
function renderApp() {
  repairState(); closeMenu(false);
  document.documentElement.classList.toggle("no-motion", !motionEnabled);
  const app = document.querySelector<HTMLElement>("#app")!;
  app.className = `${focusMode ? "focus-mode" : ""} ${sidebarCollapsed ? "sidebar-collapsed" : ""} view-${currentView}`;
  const content = currentView === "tasks" ? renderTasks() : currentView === "settings" ? renderSettings() : renderNotesPanel() + renderEditor();
  app.innerHTML = `${renderSidebar()}${content}${renderDialogLayer()}${renderHelp()}<div id="menu-layer">${renderMenu()}</div>`;
  updateSaveStatus(); bindEvents();
}

function refreshNoteList() {
  const list = document.querySelector<HTMLElement>("#note-list"); if (!list) return;
  const scroll = list.scrollTop; list.innerHTML = renderNoteRows(); list.scrollTop = scroll; bindNoteRows();
  const count = document.querySelector<HTMLElement>("#note-count"); if (count) count.textContent = `${visibleNotes().length} notes`;
}
function focusTitle() { requestAnimationFrame(() => { const title = document.querySelector<HTMLInputElement>("#title-input"); title?.focus(); title?.select(); }); }
function createNoteAndFocusTitle(folderId?: string) {
  const target = folderId && state.folders.some((folder) => folder.id === folderId) ? folderId : currentView === "notes" && state.folders.some((folder) => folder.id === state.selectedFolderId) ? state.selectedFolderId : "inbox";
  const note: Note = { id: uid("note"), folderId: target, title: "Untitled", content: "", updated: now(), status: "active" };
  state.notes.push(note); state.selectedFolderId = target; state.selectedNoteId = note.id; currentView = "notes"; searchQuery = ""; slashOpen = false; void saveState(false); renderApp(); focusTitle();
}
function openFolderDialog(parentId?: string) {
  if (parentId) state.selectedFolderId = parentId;
  const dialog = document.querySelector<HTMLDialogElement>("#folder-dialog")!; dialog.showModal(); requestAnimationFrame(() => document.querySelector<HTMLInputElement>("#folder-name")?.focus());
}
function createFolder(name: string) {
  const parent = currentView === "notes" && currentFolder().id !== "inbox" ? currentFolder() : null;
  const folder: Folder = { id: uid("folder"), name: name.trim() || "New folder", parentId: parent?.id ?? null, open: true };
  if (parent) parent.open = true; state.folders.push(folder); state.selectedFolderId = folder.id; currentView = "notes"; state.selectedNoteId = ""; void saveState(false); renderApp();
}
function setView(view: View) {
  currentView = view; focusMode = false; searchQuery = ""; repairState(); renderApp();
  if (view === "settings" && isDesktopApp && !storageInfo) void loadStorageInfo();
}
async function loadStorageInfo() {
  try { storageInfo = await invoke<StorageInfo>("get_storage_info"); storageError = ""; }
  catch (error) { storageError = `Could not read storage information: ${String(error)}`; }
  if (currentView === "settings") renderApp();
}

function noteMenuItems(note: Note): MenuItem[] {
  const common: MenuItem[] = [
    { label: "Rename", icon: "ph-pencil-simple", hint: "F2", action: `note:rename:${note.id}` },
    { label: note.pinned ? "Unpin" : "Pin", icon: note.pinned ? "ph-push-pin-slash" : "ph-push-pin", action: `note:pin:${note.id}` },
    { label: "Duplicate", icon: "ph-copy", hint: `${modLabel}+D`, action: `note:duplicate:${note.id}` },
    { label: "Move to folder…", icon: "ph-folder-notch-open", action: `note:move:${note.id}` },
    { separator: true },
    { label: "Export Markdown", icon: "ph-export", action: `note:export:${note.id}` },
  ];
  if (note.status === "active") common.push({ label: "Archive", icon: "ph-archive-tray", hint: `${modLabel}+E`, action: `note:archive:${note.id}` }, { label: "Move to Trash", icon: "ph-trash", danger: true, action: `note:trash:${note.id}` });
  if (note.status === "archived") common.push({ label: "Restore", icon: "ph-arrow-counter-clockwise", action: `note:restore:${note.id}` }, { label: "Move to Trash", icon: "ph-trash", danger: true, action: `note:trash:${note.id}` });
  if (note.status === "trash") common.push({ label: "Restore", icon: "ph-arrow-counter-clockwise", action: `note:restore:${note.id}` }, { label: "Delete permanently", icon: "ph-trash-simple", danger: true, action: `note:delete:${note.id}` });
  return common;
}
function folderMenuItems(folder: Folder): MenuItem[] {
  return [{ label: "New note", icon: "ph-note-pencil", hint: `${modLabel}+N`, action: `folder:new-note:${folder.id}` }, { label: "New subfolder", icon: "ph-folder-plus", action: `folder:new-folder:${folder.id}` }, { separator: true }, { label: "Rename", icon: "ph-pencil-simple", hint: "F2", disabled: folder.id === "inbox", action: `folder:rename:${folder.id}` }, { label: "Delete folder", icon: "ph-trash", danger: true, disabled: folder.id === "inbox", action: `folder:delete:${folder.id}` }];
}
function todoMenuItems(todo: Todo): MenuItem[] { return [{ label: todo.completed ? "Reopen" : "Complete", icon: todo.completed ? "ph-arrow-counter-clockwise" : "ph-check", action: `todo:toggle:${todo.id}` }, { label: "Edit", icon: "ph-pencil-simple", action: `todo:edit:${todo.id}` }, { label: "Duplicate", icon: "ph-copy", action: `todo:duplicate:${todo.id}` }, { separator: true }, { label: "Delete", icon: "ph-trash", danger: true, action: `todo:delete:${todo.id}` }]; }
function openMenu(items: MenuItem[], trigger: HTMLElement | null, x?: number, y?: number) {
  const rect = trigger?.getBoundingClientRect(); const menuWidth = 230; const estimatedHeight = items.length * 39 + 16;
  const left = Math.max(8, Math.min(x ?? rect?.left ?? 8, window.innerWidth - menuWidth - 8)); const top = Math.max(8, Math.min(y ?? rect?.bottom ?? 8, window.innerHeight - estimatedHeight - 8));
  menuState = { items, x: left, y: top, trigger }; const layer = document.querySelector<HTMLElement>("#menu-layer"); if (layer) { layer.innerHTML = renderMenu(); bindMenu(); requestAnimationFrame(() => document.querySelector<HTMLButtonElement>("#context-menu button:not(:disabled)")?.focus()); }
}
function closeMenu(returnFocus = true) { const trigger = menuState?.trigger; menuState = null; const layer = document.querySelector<HTMLElement>("#menu-layer"); if (layer) layer.innerHTML = ""; if (returnFocus) trigger?.focus(); }
function bindMenu() {
  const menu = document.querySelector<HTMLElement>("#context-menu"); if (!menu) return;
  menu.querySelectorAll<HTMLButtonElement>("[data-menu-index]").forEach((button) => button.addEventListener("click", () => { const item = menuState?.items[Number(button.dataset.menuIndex)]; closeMenu(false); if (item?.action) void performMenuAction(item.action); }));
  menu.addEventListener("keydown", (event) => {
    const enabled = [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]; const current = enabled.indexOf(document.activeElement as HTMLButtonElement); let next = current;
    if (event.key === "ArrowDown") next = (current + 1) % enabled.length; else if (event.key === "ArrowUp") next = (current - 1 + enabled.length) % enabled.length; else if (event.key === "Home") next = 0; else if (event.key === "End") next = enabled.length - 1; else if (event.key === "Escape") { event.preventDefault(); closeMenu(); return; } else return;
    event.preventDefault(); enabled[next]?.focus();
  });
}

async function performMenuAction(action: string) {
  const [kind, verb, id] = action.split(":");
  if (kind === "list" && verb === "sort") { sortNewest = !sortNewest; renderApp(); return; }
  if (kind === "note") {
    const note = state.notes.find((item) => item.id === id); if (!note) return;
    if (verb === "rename") {
      if (note.status !== "active") { const name = window.prompt("Rename note", note.title || "Untitled")?.trim(); if (!name) return; note.title = name; note.updated = now(); await saveState(false); renderApp(); return; }
      state.selectedNoteId = note.id; currentView = "notes"; state.selectedFolderId = note.folderId; renderApp(); focusTitle(); return;
    }
    if (verb === "pin") note.pinned = !note.pinned;
    if (verb === "duplicate") { const copy = { ...note, id: uid("note"), title: `${note.title || "Untitled"} copy`, updated: now(), status: "active" as NoteStatus }; state.notes.push(copy); state.selectedNoteId = copy.id; state.selectedFolderId = copy.folderId; currentView = "notes"; }
    if (verb === "move") { state.selectedNoteId = note.id; const dialog = document.querySelector<HTMLDialogElement>("#move-dialog")!; const select = document.querySelector<HTMLSelectElement>("#move-folder")!; select.value = note.folderId; dialog.showModal(); requestAnimationFrame(() => select.focus()); return; }
    if (verb === "archive") note.status = "archived";
    if (verb === "trash") note.status = "trash";
    if (verb === "restore") note.status = "active";
    if (verb === "delete" && window.confirm(`Permanently delete “${note.title || "Untitled"}”? This cannot be undone.`)) state.notes = state.notes.filter((item) => item.id !== note.id); else if (verb === "delete") return;
    if (verb === "export") { await exportNote(note); return; }
    note.updated = now(); repairState(); await saveState(false); renderApp();
  }
  if (kind === "folder") {
    const folder = state.folders.find((item) => item.id === id); if (!folder) return;
    if (verb === "new-note") { createNoteAndFocusTitle(folder.id); return; }
    if (verb === "new-folder") { state.selectedFolderId = folder.id; currentView = "notes"; renderApp(); openFolderDialog(folder.id); return; }
    if (verb === "rename") { const name = window.prompt("Rename folder", folder.name)?.trim(); if (!name) return; folder.name = name; }
    if (verb === "delete") {
      if (folder.id === "inbox" || !window.confirm(`Delete “${folder.name}” and its subfolders? Notes inside will move to Trash.`)) return;
      const ids = descendants(folder.id); state.notes.filter((note) => ids.includes(note.folderId)).forEach((note) => { note.status = "trash"; note.updated = now(); }); state.folders = state.folders.filter((item) => !ids.includes(item.id)); state.selectedFolderId = "inbox";
    }
    repairState(); await saveState(false); renderApp();
  }
  if (kind === "todo") {
    const todo = state.todos.find((item) => item.id === id); if (!todo) return;
    if (verb === "toggle") todo.completed = !todo.completed;
    if (verb === "edit") { editingTodoId = todo.id; renderApp(); focusTodoEditor(todo.id); return; }
    if (verb === "duplicate") state.todos.push({ ...todo, id: uid("todo"), text: `${todo.text} copy`, created: now(), updated: now() });
    if (verb === "delete") state.todos = state.todos.filter((item) => item.id !== todo.id);
    todo.updated = now(); await saveState(false); renderApp();
  }
}
async function exportNote(note: Note) {
  if (isDesktopApp) { try { await invoke<string>("export_note", { title: note.title || "Untitled", contents: note.content }); } catch (error) { window.alert(`Could not export note: ${String(error)}`); } return; }
  const blob = new Blob([`# ${note.title || "Untitled"}\n\n${note.content}`], { type: "text/markdown" }); const link = document.createElement("a"); link.href = URL.createObjectURL(blob); link.download = `${(note.title || "Untitled").replace(/[\\/:*?\"<>|]/g, "-")}.md`; link.click(); URL.revokeObjectURL(link.href);
}

function toggleTodo(id: string) { const todo = state.todos.find((item) => item.id === id); if (!todo) return; todo.completed = !todo.completed; todo.updated = now(); void saveState(false); renderApp(); }
function focusTodoEditor(id: string) { requestAnimationFrame(() => { const input = document.querySelector<HTMLInputElement>(`[data-edit-todo="${CSS.escape(id)}"]`); input?.focus(); input?.select(); }); }
function commitTodoEdit(input: HTMLInputElement) { const todo = state.todos.find((item) => item.id === input.dataset.editTodo); if (!todo) return; const text = input.value.trim(); if (text) { todo.text = text; todo.updated = now(); } editingTodoId = ""; void saveState(false); renderApp(); }
function updateSlashMenu() { const menu = document.querySelector<HTMLElement>("#slash-menu"); if (!menu) return; menu.classList.toggle("is-open", slashOpen); menu.querySelectorAll(".slash-command").forEach((item, index) => item.classList.toggle("is-active", index === slashIndex)); }
function insertCommand(index: number) { const editor = document.querySelector<HTMLTextAreaElement>("#markdown-editor"); if (!editor) return; const cursor = editor.selectionStart; const lineStart = editor.value.lastIndexOf("\n", cursor - 1) + 1; const slash = editor.value.lastIndexOf("/", cursor); const start = slash >= lineStart ? slash : cursor; editor.setRangeText(commands[index].value, start, cursor, "end"); if (commands[index].value.includes("\n\n")) editor.selectionStart = editor.selectionEnd = start + 4; slashOpen = false; updateSlashMenu(); editor.focus(); updateNote(editor); }
function insertFormatting(value: string, wrap = false, link = false) { const editor = document.querySelector<HTMLTextAreaElement>("#markdown-editor"); if (!editor) return; const start = editor.selectionStart; const end = editor.selectionEnd; const selected = editor.value.slice(start, end); const replacement = wrap ? link ? `[${selected || "link text"}](url)` : `${value}${selected}${value}` : value; editor.setRangeText(replacement, start, end, "end"); editor.focus(); updateNote(editor); }
function updateNote(editor: HTMLTextAreaElement) {
  const note = selectedNote(); if (!note || note.status !== "active") return;
  const normalized = editor.value.replace(/^(\s*)-\s*\[\](?=\s|$)/gm, "$1- [ ]"); if (normalized !== editor.value) { const cursor = editor.selectionStart + normalized.length - editor.value.length; editor.value = normalized; editor.selectionStart = editor.selectionEnd = cursor; }
  note.content = editor.value; note.updated = now(); const count = document.querySelector<HTMLElement>("#word-count"); if (count) count.textContent = `${wordCount(note.content)} words`; scheduleSave();
  const cursor = editor.selectionStart; const lineStart = editor.value.lastIndexOf("\n", cursor - 1) + 1; slashOpen = /^\/[a-z-]*$/i.test(editor.value.slice(lineStart, cursor)); if (slashOpen) slashIndex = 0; updateSlashMenu(); refreshNoteList();
}

function bindNoteRows() {
  document.querySelectorAll<HTMLElement>("[data-note-id]").forEach((row) => {
    row.addEventListener("click", (event) => { if ((event.target as HTMLElement).closest("[data-note-menu]")) return; state.selectedNoteId = row.dataset.noteId!; renderApp(); });
    row.addEventListener("contextmenu", (event) => { event.preventDefault(); const note = state.notes.find((item) => item.id === row.dataset.noteId); if (note) openMenu(noteMenuItems(note), row, event.clientX, event.clientY); });
    row.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); state.selectedNoteId = row.dataset.noteId!; renderApp(); } if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); const rows = [...document.querySelectorAll<HTMLElement>("[data-note-id]")]; const index = rows.indexOf(row); const next = rows[index + (event.key === "ArrowDown" ? 1 : -1)]; if (next) { state.selectedNoteId = next.dataset.noteId!; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-note-id="${CSS.escape(state.selectedNoteId)}"]`)?.focus()); } } });
  });
  document.querySelectorAll<HTMLElement>("[data-note-menu]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); const note = state.notes.find((item) => item.id === button.dataset.noteMenu); if (note) openMenu(noteMenuItems(note), button); }));
}
function bindEvents() {
  bindNoteRows();
  document.querySelectorAll<HTMLElement>("[data-folder-id]").forEach((row) => {
    const activate = () => { currentView = "notes"; state.selectedFolderId = row.dataset.folderId!; repairState(); void saveState(false); renderApp(); };
    row.addEventListener("click", (event) => { if (!(event.target as HTMLElement).closest("[data-toggle-folder],[data-folder-menu]")) activate(); });
    row.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); activate(); } });
    row.addEventListener("contextmenu", (event) => { event.preventDefault(); const folder = state.folders.find((item) => item.id === row.dataset.folderId); if (folder) openMenu(folderMenuItems(folder), row, event.clientX, event.clientY); });
  });
  document.querySelectorAll<HTMLElement>("[data-toggle-folder]").forEach((toggle) => toggle.addEventListener("click", (event) => { event.stopPropagation(); const folder = state.folders.find((item) => item.id === toggle.dataset.toggleFolder); if (folder) { folder.open = !folder.open; void saveState(false); renderApp(); } }));
  document.querySelectorAll<HTMLElement>("[data-folder-menu]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); const folder = state.folders.find((item) => item.id === button.dataset.folderMenu); if (folder) openMenu(folderMenuItems(folder), button); }));
  document.querySelector("#folder-tree")?.addEventListener("contextmenu", (rawEvent) => { const event = rawEvent as MouseEvent; if ((event.target as HTMLElement).closest("[data-folder-id]")) return; event.preventDefault(); openMenu([{ label: "New note", icon: "ph-note-pencil", action: "folder:new-note:inbox" }, { label: "New folder", icon: "ph-folder-plus", action: "folder:new-folder:inbox" }], event.currentTarget as HTMLElement, event.clientX, event.clientY); });
  document.querySelectorAll<HTMLElement>("[data-view]").forEach((button) => button.addEventListener("click", () => setView(button.dataset.view as View)));
  document.querySelectorAll<HTMLElement>("[data-go-inbox]").forEach((button) => button.addEventListener("click", () => { state.selectedFolderId = "inbox"; setView("notes"); }));
  document.querySelector("#wordmark")?.addEventListener("click", () => { state.selectedFolderId = "inbox"; setView("notes"); });
  document.querySelector("#new-note")?.addEventListener("click", () => createNoteAndFocusTitle());
  document.querySelectorAll("[data-create-note]").forEach((button) => button.addEventListener("click", () => createNoteAndFocusTitle()));
  document.querySelector("#new-folder")?.addEventListener("click", () => openFolderDialog());
  document.querySelector(".sidebar-toggle")?.addEventListener("click", () => { sidebarCollapsed = !sidebarCollapsed; renderApp(); });
  document.querySelector("#folder-form")?.addEventListener("submit", (event) => { event.preventDefault(); const input = document.querySelector<HTMLInputElement>("#folder-name")!; createFolder(input.value); });
  document.querySelector("#move-form")?.addEventListener("submit", (event) => { event.preventDefault(); const note = selectedNote(); const select = document.querySelector<HTMLSelectElement>("#move-folder"); if (note && select) { note.folderId = select.value; note.status = "active"; note.updated = now(); state.selectedFolderId = select.value; currentView = "notes"; void saveState(false); renderApp(); } });
  document.querySelectorAll<HTMLElement>("[data-close-dialog]").forEach((button) => button.addEventListener("click", () => button.closest("dialog")?.close()));
  document.querySelector("#list-menu")?.addEventListener("click", (event) => openMenu([{ label: sortNewest ? "Sort oldest first" : "Sort newest first", icon: "ph-sort-ascending", action: "list:sort:" }, { separator: true }, { label: "New note here", icon: "ph-note-pencil", hint: `${modLabel}+N`, action: `folder:new-note:${state.selectedFolderId}` }], event.currentTarget as HTMLElement));
  document.querySelector("#editor-menu")?.addEventListener("click", (event) => { const note = selectedNote(); if (note) openMenu(noteMenuItems(note), event.currentTarget as HTMLElement); });
  document.querySelectorAll<HTMLElement>("[data-note-action]").forEach((button) => button.addEventListener("click", () => { const note = selectedNote(); if (note) void performMenuAction(`note:${button.dataset.noteAction}:${note.id}`); }));
  const search = document.querySelector<HTMLInputElement>("#search-input"); search?.addEventListener("input", () => { searchQuery = search.value; repairState(); refreshNoteList(); });
  const title = document.querySelector<HTMLInputElement>("#title-input"); title?.addEventListener("input", () => { const note = selectedNote(); if (!note) return; note.title = title.value; note.updated = now(); scheduleSave(); refreshNoteList(); });
  const editor = document.querySelector<HTMLTextAreaElement>("#markdown-editor"); editor?.addEventListener("input", () => updateNote(editor)); editor?.addEventListener("keydown", handleEditorKeydown);
  document.querySelectorAll<HTMLElement>("[data-command-index]").forEach((button) => button.addEventListener("click", () => insertCommand(Number(button.dataset.commandIndex))));
  document.querySelectorAll<HTMLButtonElement>("[data-insert]").forEach((button) => button.addEventListener("click", () => insertFormatting(button.dataset.insert!)));
  document.querySelectorAll<HTMLButtonElement>("[data-wrap]").forEach((button) => button.addEventListener("click", () => insertFormatting(button.dataset.wrap!, true, button.hasAttribute("data-link"))));
  const toggleFocus = () => { focusMode = !focusMode; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>("#markdown-editor")?.focus()); };
  document.querySelector("#focus-mode")?.addEventListener("click", toggleFocus); document.querySelector("#expand-editor")?.addEventListener("click", toggleFocus);
  document.querySelector("#toolbar-more")?.addEventListener("click", () => { slashOpen = !slashOpen; slashIndex = 0; updateSlashMenu(); });
  bindTaskEvents(); bindSettingsEvents();
  document.querySelector("#close-help")?.addEventListener("click", () => { helpOpen = false; renderApp(); });
  document.querySelector("#help-overlay")?.addEventListener("click", (event) => { if (event.target === event.currentTarget) { helpOpen = false; renderApp(); } });
}
function handleEditorKeydown(event: KeyboardEvent) {
  const editor = event.currentTarget as HTMLTextAreaElement;
  if (slashOpen && ["ArrowDown","ArrowUp","Enter","Escape"].includes(event.key)) { event.preventDefault(); if (event.key === "ArrowDown") slashIndex = (slashIndex + 1) % commands.length; if (event.key === "ArrowUp") slashIndex = (slashIndex - 1 + commands.length) % commands.length; if (event.key === "Enter") { insertCommand(slashIndex); return; } if (event.key === "Escape") slashOpen = false; updateSlashMenu(); }
  if (event.key === "Tab" && !slashOpen) { event.preventDefault(); editor.setRangeText("  ", editor.selectionStart, editor.selectionEnd, "end"); updateNote(editor); }
}
function bindTaskEvents() {
  document.querySelector("#quick-task-form")?.addEventListener("submit", (event) => { event.preventDefault(); const input = document.querySelector<HTMLInputElement>("#quick-task-input")!; const text = input.value.trim(); if (!text) return; const timestamp = now(); state.todos.unshift({ id: uid("todo"), text, completed: false, created: timestamp, updated: timestamp }); void saveState(false); renderApp(); requestAnimationFrame(() => document.querySelector<HTMLInputElement>("#quick-task-input")?.focus()); });
  document.querySelectorAll<HTMLElement>("[data-toggle-todo]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); toggleTodo(button.dataset.toggleTodo!); }));
  document.querySelectorAll<HTMLElement>("[data-todo-id]").forEach((row) => { row.addEventListener("dblclick", () => { editingTodoId = row.dataset.todoId!; renderApp(); focusTodoEditor(editingTodoId); }); row.addEventListener("contextmenu", (event) => { event.preventDefault(); const todo = state.todos.find((item) => item.id === row.dataset.todoId); if (todo) openMenu(todoMenuItems(todo), row, event.clientX, event.clientY); }); row.addEventListener("keydown", handleTaskKeydown); });
  document.querySelectorAll<HTMLElement>("[data-todo-menu]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); const todo = state.todos.find((item) => item.id === button.dataset.todoMenu); if (todo) openMenu(todoMenuItems(todo), button); }));
  document.querySelectorAll<HTMLInputElement>("[data-edit-todo]").forEach((input) => { input.addEventListener("keydown", (event) => { if (event.key === "Enter") commitTodoEdit(input); if (event.key === "Escape") { editingTodoId = ""; renderApp(); } }); input.addEventListener("blur", () => { if (editingTodoId) commitTodoEdit(input); }); });
  document.querySelector("#toggle-completed")?.addEventListener("click", () => { completedCollapsed = !completedCollapsed; renderApp(); });
  document.querySelector("#clear-completed")?.addEventListener("click", () => { if (window.confirm("Clear all completed tasks? This cannot be undone.")) { state.todos = state.todos.filter((todo) => !todo.completed); void saveState(false); renderApp(); } });
}
function handleTaskKeydown(event: KeyboardEvent) {
  if ((event.target as HTMLElement).matches("input")) return; const row = event.currentTarget as HTMLElement; const id = row.dataset.todoId!; const rows = [...document.querySelectorAll<HTMLElement>("[data-todo-id]")]; const index = rows.indexOf(row);
  if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); rows[index + (event.key === "ArrowDown" ? 1 : -1)]?.focus(); }
  if (event.key === " ") { event.preventDefault(); toggleTodo(id); }
  if (event.key === "Enter") { event.preventDefault(); editingTodoId = id; renderApp(); focusTodoEditor(id); }
  if (event.key === "Delete" && window.confirm("Delete this task?")) { state.todos = state.todos.filter((todo) => todo.id !== id); void saveState(false); renderApp(); }
}
function bindSettingsEvents() {
  document.querySelector("#motion-toggle")?.addEventListener("change", (event) => { motionEnabled = (event.target as HTMLInputElement).checked; localStorage.setItem(MOTION_KEY, String(motionEnabled)); document.documentElement.classList.toggle("no-motion", !motionEnabled); });
  document.querySelector("#show-help")?.addEventListener("click", () => { helpOpen = true; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLButtonElement>("#close-help")?.focus()); });
  document.querySelector("#create-backup")?.addEventListener("click", async (event) => { const button = event.currentTarget as HTMLButtonElement; button.disabled = true; button.innerHTML = '<i class="ph ph-circle-notch"></i>Creating…'; try { const path = await invoke<string>("create_backup"); window.alert(`Backup created:\n${path}`); } catch (error) { window.alert(`Could not create backup: ${String(error)}`); } finally { renderApp(); } });
}

function isTypingTarget(target: EventTarget | null) { const element = target as HTMLElement | null; return !!element?.closest("input, textarea, select, [contenteditable=true]"); }
document.addEventListener("pointerdown", (event) => { if (menuState && !(event.target as HTMLElement).closest("#context-menu") && !(event.target as HTMLElement).closest("[data-note-menu],[data-folder-menu],[data-todo-menu],#editor-menu,#list-menu")) closeMenu(false); });
window.addEventListener("blur", () => closeMenu(false)); window.addEventListener("resize", () => closeMenu(false));
document.addEventListener("keydown", (event) => {
  const command = event.metaKey || event.ctrlKey; const key = event.key.toLowerCase(); const typing = isTypingTarget(event.target);
  if (event.key === "Escape") { if (menuState) { event.preventDefault(); closeMenu(); return; } const openDialog = document.querySelector<HTMLDialogElement>("dialog[open]"); if (openDialog) { event.preventDefault(); openDialog.close(); return; } if (helpOpen) { event.preventDefault(); helpOpen = false; renderApp(); return; } if (focusMode) { event.preventDefault(); focusMode = false; renderApp(); return; } }
  if (command && key === "n") { event.preventDefault(); if (event.shiftKey) openFolderDialog(); else createNoteAndFocusTitle(); return; }
  if (command && key === "k") { event.preventDefault(); if (!document.querySelector("#search-input")) { currentView = "notes"; renderApp(); } requestAnimationFrame(() => document.querySelector<HTMLInputElement>("#search-input")?.focus()); return; }
  if (command && key === "s") { event.preventDefault(); void saveState(); return; }
  if (command && event.shiftKey && key === "f") { event.preventDefault(); focusMode = !focusMode; if (currentView !== "notes") { currentView = "notes"; repairState(); } renderApp(); return; }
  if (command && ((event.shiftKey && key === "t") || key === "2")) { event.preventDefault(); setView("tasks"); return; }
  if (command && key === "1") { event.preventDefault(); state.selectedFolderId = "inbox"; setView("notes"); return; }
  if (!typing && command && key === "d") { const note = selectedNote(); if (note) { event.preventDefault(); void performMenuAction(`note:duplicate:${note.id}`); } return; }
  if (!typing && command && key === "e") { const note = selectedNote(); if (note?.status === "active") { event.preventDefault(); void performMenuAction(`note:archive:${note.id}`); } return; }
  if (!typing && event.key === "F2") { const note = selectedNote(); if (note && ["notes","archived","trash"].includes(currentView)) { event.preventDefault(); void performMenuAction(`note:rename:${note.id}`); } }
  if (!typing && event.key === "?") { event.preventDefault(); helpOpen = true; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLButtonElement>("#close-help")?.focus()); }
});

renderApp();
void restoreDesktopWorkspace();
