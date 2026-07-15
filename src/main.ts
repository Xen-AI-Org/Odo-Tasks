import "@phosphor-icons/web/regular/style.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./styles.css";

type NoteStatus = "active" | "archived" | "trash";
type View = "notes" | "tasks" | "journal" | "settings" | "archived" | "trash";
type SortMode = "newest" | "oldest" | "manual";
type Folder = { id: string; name: string; parentId: string | null; open: boolean; icon?: string };
type Note = { id: string; folderId: string; title: string; content: string; updated: string; status: NoteStatus; pinned?: boolean; revision: number };
type Priority = "low" | "medium" | "high" | "urgent";
type Todo = { id: string; text: string; completed: boolean; created: string; updated: string; categoryId: string; priority: Priority; effort: number; color: string; scheduledStart: string | null; durationMinutes: number };
type TodoCategory = { id: string; name: string; color: string; icon?: string };
type JournalEntry = { id: string; dateKey: string; content: string; created: string; updated: string };
type PlannerView = "1" | "3" | "4" | "7";
type Workspace = { folders: Folder[]; notes: Note[]; todos: Todo[]; todoCategories: TodoCategory[]; journalEntries: JournalEntry[]; selectedFolderId: string; selectedNoteId: string; sortMode: SortMode; plannerView: PlannerView };
type StorageInfo = { databasePath: string; backupDirectory: string };
type SavePhase = "idle" | "saving" | "saved" | "error";
type MenuItem = { label?: string; icon?: string; hint?: string; action?: string; disabled?: boolean; danger?: boolean; separator?: boolean };
type MenuState = { items: MenuItem[]; x: number; y: number; trigger: HTMLElement | null } | null;
type DialogOptions = { kind: "confirm" | "prompt" | "notice"; title: string; message: string; confirmLabel?: string; cancelLabel?: string; destructive?: boolean; label?: string; initialValue?: string; path?: string; validate?: (value: string) => string | null };
type DialogState = { options: DialogOptions; trigger: HTMLElement | null; resolve: (value: boolean | string | null) => void; error: string } | null;

const STORAGE_KEY = "odo-notes-workspace-v2";
const MOTION_KEY = "odo-motion-enabled";
const isDesktopApp = "__TAURI_INTERNALS__" in window;
const detachedNoteId = new URLSearchParams(location.search).get("note");
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
  { id: "welcome", folderId: "inbox", title: "Welcome to Odo", updated: now(), status: "active", pinned: true, revision: 0, content: "## A calm place for your work\n\nCapture ideas, organize projects, and keep your day moving.\n\n- Press Ctrl+N for a new note\n- Press Ctrl+2 for Tasks\n- Type / on a new line for blocks" },
];
const starterWorkspace = (): Workspace => ({ folders: structuredClone(starterFolders), notes: structuredClone(starterNotes), todos: [], todoCategories: [{ id: "inbox", name: "Inbox", color: "#7b8e7c", icon: "ph-tray" }], journalEntries: [], selectedFolderId: "inbox", selectedNoteId: "welcome", sortMode: "newest", plannerView: "3" });

const escapeHtml = (value: string) => value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/\"/g, "&quot;").replace(/'/g, "&#039;");
const attr = escapeHtml;
function normalizeWorkspace(input: Partial<Workspace> | null | undefined): Workspace {
  const fallback = starterWorkspace();
  const folders = Array.isArray(input?.folders) ? input.folders : fallback.folders;
  if (!folders.some((folder) => folder.id === "inbox")) folders.unshift({ id: "inbox", name: "Inbox", parentId: null, open: true, icon: "ph-tray" });
  const notes = (Array.isArray(input?.notes) ? input.notes : fallback.notes).map((note) => ({ ...note, revision: note.revision ?? 0 }));
  const sortMode: SortMode = input?.sortMode === "manual" || input?.sortMode === "oldest" ? input.sortMode : "newest";
  const categories = Array.isArray(input?.todoCategories) && input.todoCategories.length ? input.todoCategories : [{ id: "inbox", name: "Inbox", color: "#7b8e7c", icon: "ph-tray" }];
  const todos = (Array.isArray(input?.todos) ? input.todos : []).map((todo) => ({ ...todo, categoryId: todo.categoryId || "inbox", priority: (todo.priority || "medium") as Priority, effort: todo.effort || 2, color: todo.color || "", scheduledStart: todo.scheduledStart || null, durationMinutes: todo.durationMinutes || 30 }));
  const plannerView: PlannerView = ["1", "3", "4", "7"].includes(input?.plannerView || "") ? input!.plannerView as PlannerView : "3";
  const journalEntries = (Array.isArray(input?.journalEntries) ? input.journalEntries : []).filter((entry): entry is JournalEntry => !!entry && typeof entry.dateKey === "string").map((entry) => ({ id: entry.id || uid("journal"), dateKey: entry.dateKey, content: entry.content || "", created: entry.created || now(), updated: entry.updated || entry.created || now() }));
  return { folders, notes, todos, todoCategories: categories, journalEntries, selectedFolderId: input?.selectedFolderId || "inbox", selectedNoteId: input?.selectedNoteId || "", sortMode, plannerView };
}
function initialState(): Workspace {
  if (!isDesktopApp) try { const saved = localStorage.getItem(STORAGE_KEY); if (saved) return normalizeWorkspace(JSON.parse(saved) as Workspace); } catch { localStorage.removeItem(STORAGE_KEY); }
  return starterWorkspace();
}

let state = initialState();
let currentView: View = "notes";
let searchQuery = "";
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
let dialogState: DialogState = null;
let workspaceReady = !isDesktopApp;
const detachedNoteIds = new Set<string>();
let newRowId = "";
let storageInfo: StorageInfo | null = null;
let storageError = "";
let motionEnabled = localStorage.getItem(MOTION_KEY) !== "false";
let draggingNoteId = "";
let suppressRowActivationUntil = 0;
let sortInsertionTargetId = "";
let sortInsertionSide: "before" | "after" = "before";
let dragImage: HTMLElement | null = null;
let plannerDate = new Date(); plannerDate.setHours(0, 0, 0, 0);
let taskMenuTodoId = "";
let plannerDragTodoId = "";
let plannerPopover: { x: number; y: number; returnId: string } | null = null;
let plannerPointerDragging = false;
let plannerPointerDrop: { date: string; minute: number } | null = null;
let selectedJournalDate = "";
let journalOpening = false;
let journalSaveTimer = 0;

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
  const notes = state.notes.filter((note) => note.status === statusForView()).filter((note) => !folderIds || folderIds.includes(note.folderId)).filter((note) => `${note.title} ${note.content}`.toLowerCase().includes(searchQuery.toLowerCase()));
  if (state.sortMode === "manual") return notes;
  return notes.sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    const delta = new Date(b.updated).getTime() - new Date(a.updated).getTime();
    return state.sortMode === "newest" ? delta : -delta;
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
  workspaceReady = true; repairState(); renderApp();
}

function renderFolder(folder: Folder, depth = 0): string {
  const children = state.folders.filter((candidate) => candidate.parentId === folder.id);
  const toggle = children.length ? `<button class="folder-toggle" data-toggle-folder="${attr(folder.id)}" aria-label="${folder.open ? "Collapse" : "Expand"} ${attr(folder.name)}"><i class="ph ph-caret-${folder.open ? "down" : "right"}"></i></button>` : '<span class="folder-toggle-spacer"></span>';
  return `<div class="folder-branch"><div class="folder-row ${currentView === "notes" && state.selectedFolderId === folder.id ? "is-selected" : ""}" data-folder-id="${attr(folder.id)}" data-drop-kind="folder" data-drop-id="${attr(folder.id)}" style="--depth:${depth}" role="button" tabindex="0" aria-dropeffect="move" aria-label="Move a note to ${attr(folder.name)}. ${folderCount(folder.id)} notes">${toggle}<i class="ph ${folder.icon ?? "ph-folder"} folder-icon"></i><span class="folder-name">${escapeHtml(folder.name)}</span><span class="folder-count">${folderCount(folder.id)}</span><span class="drop-cue" aria-hidden="true">Move here</span><button class="row-more" data-folder-menu="${attr(folder.id)}" aria-label="Folder actions"><i class="ph ph-dots-three"></i></button></div>${folder.open ? children.map((child) => renderFolder(child, depth + 1)).join("") : ""}</div>`;
}
function renderSidebar() {
  const remaining = state.todos.filter((todo) => !todo.completed).length;
  return `<aside class="folders-panel" aria-label="Workspace navigation"><header class="brand-row"><button class="wordmark" id="wordmark" title="Go to Inbox">Odo</button><button class="icon-button sidebar-toggle" title="${sidebarCollapsed ? "Show" : "Hide"} sidebar" aria-label="${sidebarCollapsed ? "Show" : "Hide"} sidebar"><i class="ph ph-sidebar-simple"></i></button></header>
    <nav class="primary-nav"><button class="primary-link ${currentView === "notes" && state.selectedFolderId === "inbox" ? "is-selected" : ""}" data-go-inbox data-drop-kind="folder" data-drop-id="inbox" aria-dropeffect="move" aria-label="Move a note to Inbox"><i class="ph ph-tray"></i><span>Inbox</span><span class="drop-cue" aria-hidden="true">Move here</span><kbd>${modLabel}+1</kbd></button><button class="primary-link ${currentView === "tasks" ? "is-selected" : ""}" data-view="tasks"><i class="ph ph-check-square"></i><span>Tasks</span><span class="nav-count ${remaining ? "has-items" : ""}">${remaining}</span></button><button class="primary-link ${currentView === "journal" ? "is-selected" : ""}" data-view="journal" aria-label="Journal"><i class="ph ph-book-open-text"></i><span>Journal</span></button></nav>
    <div class="panel-label-row"><span>Folders</span><button class="icon-button" id="new-folder" title="New folder (${modLabel}+Shift+N)"><i class="ph ph-plus"></i></button></div><nav class="folder-tree" id="folder-tree">${state.folders.filter((folder) => folder.parentId === null && folder.id !== "inbox").map((folder) => renderFolder(folder)).join("")}</nav>
    <div class="library-links"><button class="library-link archive-drop ${currentView === "archived" ? "is-selected" : ""}" data-view="archived" data-drop-kind="archive" aria-dropeffect="move" aria-label="Archive this note"><i class="ph ph-archive-tray"></i><span>Archive</span><span class="drop-cue" aria-hidden="true">Move here</span><span>${state.notes.filter((note) => note.status === "archived").length}</span></button><button class="library-link trash-drop ${currentView === "trash" ? "is-selected" : ""}" data-view="trash" data-drop-kind="trash" aria-dropeffect="move" aria-label="Move this note to Trash"><i class="ph ph-trash"></i><span>Trash</span><span class="drop-cue" aria-hidden="true">Move here</span><span>${state.notes.filter((note) => note.status === "trash").length}</span></button></div><button class="settings-link ${currentView === "settings" ? "is-selected" : ""}" data-view="settings"><i class="ph ph-gear"></i><span>Settings</span></button></aside>`;
}
function renderNotesPanel() {
  const ordering = state.sortMode === "manual" ? "manual order" : state.sortMode === "newest" ? "newest first" : "oldest first";
  return `<section class="notes-panel"><div class="global-bar"><label class="search-box"><i class="ph ph-magnifying-glass"></i><input id="search-input" type="search" placeholder="Search notes..." value="${attr(searchQuery)}"><kbd>${modLabel}+K</kbd></label><button class="new-note-button" id="new-note" title="New note (${modLabel}+N)"><i class="ph ph-note-pencil"></i></button></div><header class="notes-header"><div><strong id="note-list-title">${escapeHtml(viewTitle())}</strong><span id="note-count">${visibleNotes().length} notes · ${ordering}</span></div><button class="sort-button" id="list-menu" title="Change note order" aria-label="Change note order"><i class="ph ph-dots-three"></i></button></header><div class="note-list" id="note-list" tabindex="-1">${renderNoteRows()}</div></section>`;
}
function renderNoteRows() {
  const notes = visibleNotes();
  if (!notes.length) return `<div class="empty-state"><span class="empty-icon"><i class="ph ${currentView === "trash" ? "ph-trash" : currentView === "archived" ? "ph-archive-tray" : "ph-note-blank"}"></i></span><strong>${searchQuery ? "No matching notes" : currentView === "trash" ? "Trash is empty" : currentView === "archived" ? "Nothing archived" : "A clear page awaits"}</strong><p>${searchQuery ? "Try a different search." : "Capture a thought and give it somewhere to grow."}</p>${currentView === "notes" && !searchQuery ? '<button class="primary-button" data-create-note>Create a note</button>' : ""}</div>`;
  return notes.map((note) => `<div class="note-row ${note.id === state.selectedNoteId ? "is-selected" : ""} ${note.id === newRowId ? "is-new" : ""}" data-note-id="${attr(note.id)}" role="button" tabindex="${note.id === state.selectedNoteId ? "0" : "-1"}" draggable="true" aria-label="${attr(note.title || "Untitled")}. Drag to a folder, Archive, or Trash."><div class="note-heading"><span class="note-title-wrap">${note.pinned ? '<i class="ph ph-push-pin pin-icon"></i>' : ""}<span class="note-title">${linkedPlainText(note.title || "Untitled")}</span></span><time>${formatListDate(note.updated)}</time><button class="row-more" data-note-menu="${attr(note.id)}" aria-label="Note actions"><i class="ph ph-dots-three"></i></button></div><span class="note-excerpt">${escapeHtml(noteExcerpt(note.content))}</span></div>`).join("");
}
function slashMenuHtml() { return `<div class="slash-menu ${slashOpen ? "is-open" : ""}" id="slash-menu" role="listbox">${commands.map((command, index) => `<button class="slash-command ${index === slashIndex ? "is-active" : ""}" data-command-index="${index}" role="option" aria-selected="${index === slashIndex}"><span class="command-icon"><i class="ph ${command.icon}"></i></span><span><strong>${command.label}</strong><small>${command.detail}</small></span>${index === 0 ? "<kbd>Enter</kbd>" : ""}</button>`).join("")}</div>`; }
type RichBlock = "P" | "H1" | "H2" | "H3" | "BLOCKQUOTE" | "PRE" | "UL" | "OL";
const blockSelector = "p,h1,h2,h3,blockquote,pre,ul,ol";
const linkableUrlPattern = /(^|[^\w@])((?:(?:https?:\/\/|www\.)[^\s<>"']+|(?:[a-z0-9](?:[a-z0-9-]{0,62}\.)+[a-z]{2,})(?::\d+)?(?:\/[^\s<>"']*)?))/gi;

function trimUrlPunctuation(value: string): string {
  let trimmed = value.replace(/[.,!?;:]+$/g, "");
  const pairs: [string, string][] = [["(", ")"], ["[", "]"], ["{", "}"]];
  for (const [open, close] of pairs) {
    while (trimmed.endsWith(close) && trimmed.split(close).length > trimmed.split(open).length) trimmed = trimmed.slice(0, -1);
  }
  return trimmed;
}
function webHref(value: string): string | null {
  try {
    const url = new URL(/^https?:\/\//i.test(value) ? value : `https://${value}`);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch { return null; }
}
function linkifyPlainUrls(root: ParentNode): boolean {
  const documentRoot = root instanceof DocumentFragment ? root.ownerDocument : root.ownerDocument ?? document;
  const walker = documentRoot.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = []; let current: Node | null; let changed = false;
  while ((current = walker.nextNode())) {
    const parent = current.parentElement;
    if (!parent || !parent.closest("a, code, pre")) nodes.push(current as Text);
  }
  nodes.forEach((node) => {
    const text = node.data; const matches: { start: number; end: number; label: string; href: string }[] = [];
    linkableUrlPattern.lastIndex = 0; let match: RegExpExecArray | null;
    while ((match = linkableUrlPattern.exec(text))) {
      const label = trimUrlPunctuation(match[2]); const href = webHref(label);
      if (!label || !href) continue;
      const start = match.index + match[1].length;
      matches.push({ start, end: start + label.length, label, href });
    }
    if (!matches.length) return;
    const fragment = documentRoot.createDocumentFragment(); let offset = 0;
    matches.forEach(({ start, end, label, href }) => {
      fragment.append(text.slice(offset, start));
      const anchor = documentRoot.createElement("a"); anchor.href = href; anchor.dataset.autoLink = "true"; anchor.rel = "noopener noreferrer"; anchor.textContent = label; fragment.append(anchor);
      offset = end;
    });
    fragment.append(text.slice(offset)); node.replaceWith(fragment); changed = true;
  });
  return changed;
}
function linkedPlainText(value: string): string {
  const template = document.createElement("template"); template.content.textContent = value; linkifyPlainUrls(template.content); return template.innerHTML;
}
function titleEditorText(editor: HTMLElement): string { return (editor.textContent ?? "").replace(/[\r\n]+/g, " ").replace(/\u00a0/g, " "); }
function selectionTextOffset(root: HTMLElement): number | null {
  const selection = window.getSelection(); if (!selection?.isCollapsed || !selection.anchorNode || !root.contains(selection.anchorNode)) return null;
  const range = document.createRange(); range.selectNodeContents(root); range.setEnd(selection.anchorNode, selection.anchorOffset); return range.toString().length;
}
function restoreSelectionTextOffset(root: HTMLElement, offset: number | null) {
  if (offset === null) return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT); let remaining = offset; let current: Node | null;
  while ((current = walker.nextNode())) {
    const length = current.textContent?.length ?? 0;
    if (remaining <= length) { const range = document.createRange(); range.setStart(current, remaining); range.collapse(true); const selection = window.getSelection(); selection?.removeAllRanges(); selection?.addRange(range); return; }
    remaining -= length;
  }
  placeCaretEnd(root);
}
function refreshAutoLinks(editor: HTMLElement) {
  const offset = selectionTextOffset(editor); let changed = false;
  editor.querySelectorAll<HTMLAnchorElement>("a[data-auto-link]").forEach((anchor) => {
    const text = anchor.textContent ?? ""; const label = trimUrlPunctuation(text); const href = webHref(label);
    if (href && label === text) { if (anchor.getAttribute("href") !== href) anchor.href = href; return; }
    anchor.replaceWith(document.createTextNode(text)); changed = true;
  });
  if (changed) editor.normalize();
  changed = linkifyPlainUrls(editor) || changed;
  if (changed) restoreSelectionTextOffset(editor, offset);
}
function shouldRefreshAutoLinks(event: InputEvent): boolean {
  const selection = window.getSelection(); const anchorNode = selection?.anchorNode;
  const anchor = anchorNode instanceof HTMLElement ? anchorNode.closest("a[data-auto-link]") : anchorNode?.parentElement?.closest("a[data-auto-link]");
  return !!anchor || event.inputType === "insertParagraph" || event.inputType === "insertLineBreak" || event.inputType === "insertFromPaste" || (event.data?.length ?? 0) > 1 || /\s/.test(event.data ?? "");
}
async function openRichLink(anchor: HTMLAnchorElement) {
  const href = webHref(anchor.href); if (!href) return;
  try { if (isDesktopApp) await openUrl(href); else window.open(href, "_blank", "noopener,noreferrer"); }
  catch (error) { console.error("Could not open link:", error); }
}
function bindTitleEditor(editor: HTMLElement, update: (value: string) => void, onEnter: () => void, onEscape: () => void) {
  const sync = () => update(titleEditorText(editor));
  editor.addEventListener("input", (event) => { if (shouldRefreshAutoLinks(event as InputEvent)) refreshAutoLinks(editor); sync(); });
  editor.addEventListener("blur", () => { refreshAutoLinks(editor); sync(); });
  editor.addEventListener("keydown", (event) => { if (event.isComposing) return; if (event.key === "Enter") { event.preventDefault(); onEnter(); } else if (event.key === "Escape") { event.preventDefault(); onEscape(); } });
  editor.addEventListener("click", (event) => { const anchor = (event.target as HTMLElement).closest<HTMLAnchorElement>("a[href]"); if (anchor) { event.preventDefault(); event.stopPropagation(); void openRichLink(anchor); } });
  editor.addEventListener("paste", (event) => { event.preventDefault(); const text = (event.clipboardData?.getData("text/plain") ?? "").replace(/\s*[\r\n]+\s*/g, " "); document.execCommand("insertText", false, text); });
}

function inlineMarkdown(markdown: string): string {
  // Escape first: documents are always modelled as text/semantic tags, never pasted HTML.
  let html = escapeHtml(markdown);
  html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
  html = html.replace(/\[([^\]]+)\]\(([^\s)]+)\)/g, '<a href="$2" rel="noopener noreferrer">$1</a>');
  html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/~~([^~]+)~~/g, "<s>$1</s>");
  html = html.replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
  html = html.replace(/(^|[^_])_([^_]+)_/g, "$1<em>$2</em>");
  const template = document.createElement("template"); template.innerHTML = html; linkifyPlainUrls(template.content); return template.innerHTML;
}
function richTask(text: string, checked: boolean) { return `<li class="rich-task ${checked ? "is-checked" : ""}"><input class="rich-task-check" type="checkbox" ${checked ? "checked" : ""} contenteditable="false" aria-label="Toggle task"><span class="rich-task-label" contenteditable="true">${text ? inlineMarkdown(text) : "<br>"}</span></li>`; }
function markdownToRich(markdown: string): string {
  const lines = markdown.replace(/\r\n?/g, "\n").split("\n"); const output: string[] = []; let index = 0;
  const paragraph = (text: string) => `<p>${inlineMarkdown(text) || "<br>"}</p>`;
  while (index < lines.length) {
    const line = lines[index];
    if (/^```/.test(line)) { const language = line.slice(3).trim(); const code: string[] = []; index++; while (index < lines.length && !/^```/.test(lines[index])) code.push(lines[index++]); if (index < lines.length) index++; output.push(`<pre data-language="${attr(language)}"><code>${escapeHtml(code.join("\n")) || "\n"}</code></pre>`); continue; }
    if (/^\s*---+\s*$/.test(line)) { output.push("<hr>"); index++; continue; }
    const heading = line.match(/^(#{1,3})\s+(.*)$/); if (heading) { output.push(`<h${heading[1].length}>${inlineMarkdown(heading[2]) || "<br>"}</h${heading[1].length}>`); index++; continue; }
    if (/^>\s?/.test(line)) { const quote: string[] = []; while (index < lines.length && /^>\s?/.test(lines[index])) quote.push(lines[index++].replace(/^>\s?/, "")); output.push(`<blockquote>${quote.map(paragraph).join("")}</blockquote>`); continue; }
    if (/^- \[[ xX]\](?:\s|$)/.test(line)) { const tasks: string[] = []; while (index < lines.length && /^- \[[ xX]\](?:\s|$)/.test(lines[index])) { const task = lines[index++].match(/^- \[([ xX])\]\s?(.*)$/)!; tasks.push(richTask(task[2], task[1].toLowerCase() === "x")); } output.push(`<ul class="rich-list rich-task-list">${tasks.join("")}</ul>`); continue; }
    if (/^-\s+/.test(line)) { const items: string[] = []; while (index < lines.length && /^-\s+/.test(lines[index])) items.push(`<li>${inlineMarkdown(lines[index++].replace(/^-\s+/, "")) || "<br>"}</li>`); output.push(`<ul class="rich-list">${items.join("")}</ul>`); continue; }
    if (/^\d+\.\s+/.test(line)) { const items: string[] = []; while (index < lines.length && /^\d+\.\s+/.test(lines[index])) items.push(`<li>${inlineMarkdown(lines[index++].replace(/^\d+\.\s+/, "")) || "<br>"}</li>`); output.push(`<ol class="rich-list">${items.join("")}</ol>`); continue; }
    if (!line.trim()) { index++; if (output.length && output[output.length - 1] !== "<p><br></p>") output.push("<p><br></p>"); continue; }
    const text: string[] = [line]; index++; while (index < lines.length && lines[index].trim() && !/^(#{1,3}\s|>|- \[[ xX]\]\s|-\s+|\d+\.\s+|```|---+$)/.test(lines[index])) text.push(lines[index++]); output.push(paragraph(text.join("\n")));
  }
  return output.join("") || "<p><br></p>";
}
function richInlineMarkdown(node: Node): string {
  if (node.nodeType === Node.TEXT_NODE) return node.textContent ?? "";
  if (!(node instanceof HTMLElement)) return "";
  const inner = [...node.childNodes].map(richInlineMarkdown).join("");
  if (node.tagName === "STRONG" || node.tagName === "B") return `**${inner}**`;
  if (node.tagName === "EM" || node.tagName === "I") return `_${inner}_`;
  if (node.tagName === "S" || node.tagName === "DEL" || node.tagName === "STRIKE") return `~~${inner}~~`;
  if (node.tagName === "CODE") return `\`${inner}\``;
  if (node.tagName === "A") return node.hasAttribute("data-auto-link") ? inner : `[${inner}](${node.getAttribute("href") || "url"})`;
  if (node.tagName === "BR") return "\n";
  return inner;
}
function normalizeRichTasks(editor: HTMLElement) {
  editor.querySelectorAll<HTMLElement>(".rich-task").forEach((item) => {
    let label = item.querySelector<HTMLElement>(".rich-task-label");
    if (!label) { label = document.createElement("span"); label.className = "rich-task-label"; label.contentEditable = "true"; item.append(label); }
    // Chromium can place text beside the nested label after a structural
    // conversion. Move it back before every model read so no task text is lost.
    [...item.childNodes].filter((node) => node !== label && !(node instanceof HTMLInputElement) && (node.nodeType === Node.TEXT_NODE || node.nodeName === "BR")).forEach((node) => label!.append(node));
    if (!label.childNodes.length) label.append(document.createElement("br"));
  });
}
function richToMarkdown(editor: HTMLElement): string {
  normalizeRichTasks(editor);
  const blocks = [...editor.children] as HTMLElement[];
  const values = blocks.map((block) => {
    const text = richInlineMarkdown(block).replace(/\u00a0/g, " ").replace(/\n+$/, "");
    if (block.tagName === "H1") return `# ${text}`; if (block.tagName === "H2") return `## ${text}`; if (block.tagName === "H3") return `### ${text}`;
    if (block.tagName === "BLOCKQUOTE") return [...block.children].map((item) => `> ${richInlineMarkdown(item).replace(/\n/g, " ")}`).join("\n");
    if (block.tagName === "PRE") { const code = block.querySelector("code")?.textContent ?? block.textContent ?? ""; const language = block.dataset.language || ""; return `\`\`\`${language}\n${code.replace(/\n$/, "")}\n\`\`\``; }
    if (block.tagName === "HR") return "---";
    if (block.tagName === "UL" || block.tagName === "OL") return [...block.children].filter((item) => item.tagName === "LI").map((item, i) => { const task = item.classList.contains("rich-task"); const label = task ? item.querySelector<HTMLElement>(".rich-task-label") : item; const content = richInlineMarkdown(label ?? item).replace(/\n/g, " "); return task ? `- [${item.querySelector<HTMLInputElement>("input")?.checked ? "x" : " "}]${content.trim() ? ` ${content.trim()}` : ""}` : block.tagName === "OL" ? `${i + 1}. ${content}` : `- ${content}`; }).join("\n");
    return text;
  }).filter((value, i, list) => value || (i > 0 && i < list.length - 1));
  return values.join("\n\n").replace(/\n{3,}/g, "\n\n").trim();
}
function editorSelection(): Selection | null { const selection = window.getSelection(); return selection?.rangeCount && selection.anchorNode && (selection.anchorNode.parentElement?.closest("[data-rich-editor]") || (selection.anchorNode as HTMLElement).closest?.("[data-rich-editor]")) ? selection : null; }
function currentRichBlock(editor: HTMLElement): HTMLElement | null {
  const selection = editorSelection(); let target = selection?.anchorNode instanceof HTMLElement ? selection.anchorNode : selection?.anchorNode?.parentElement;
  while (target && target !== editor) {
    if (target.parentElement === editor && (target.matches(blockSelector) || target.tagName === "DIV")) return target;
    if (target.tagName === "LI") return target;
    target = target.parentElement;
  }
  return null;
}
function placeCaretEnd(element: HTMLElement) { const selection = window.getSelection(); const range = document.createRange(); range.selectNodeContents(element); range.collapse(false); selection?.removeAllRanges(); selection?.addRange(range); element.focus(); }
function placeCaretInTaskLabel(label: HTMLElement) { if (!label.childNodes.length) label.append(document.createElement("br")); const selection = window.getSelection(); const range = document.createRange(); range.selectNodeContents(label); range.collapse(false); selection?.removeAllRanges(); selection?.addRange(range); label.focus(); }
function createTaskList(): HTMLElement { const list = document.createElement("ul"); list.className = "rich-list rich-task-list"; return list; }
function createTaskItem(): HTMLElement { const holder = document.createElement("template"); holder.innerHTML = richTask("", false); return holder.content.firstElementChild as HTMLElement; }
function replaceRichBlock(editor: HTMLElement, next: HTMLElement) { const block = currentRichBlock(editor); if (block) block.replaceWith(next); else editor.append(next); const label = next.querySelector<HTMLElement>(".rich-task-label"); if (label) placeCaretInTaskLabel(label); else placeCaretEnd(next); }
function applyRichBlock(type: RichBlock) {
  const editor = document.querySelector<HTMLElement>("[data-rich-editor]"); if (!editor) return; editor.focus(); const block = currentRichBlock(editor); if (type === "UL" || type === "OL") { document.execCommand(type === "UL" ? "insertUnorderedList" : "insertOrderedList"); updateRichNote(editor); return; }
  if (type === "PRE") { const pre = document.createElement("pre"); const code = document.createElement("code"); code.textContent = block?.textContent || ""; pre.append(code); replaceRichBlock(editor, pre); }
  else { document.execCommand("formatBlock", false, type); }
  updateRichNote(editor);
}
function applyRichInline(command: string) { const editor = document.querySelector<HTMLElement>("[data-rich-editor]"); if (!editor) return; editor.focus(); if (command === "link") { const href = "https://"; document.execCommand("createLink", false, href); } else document.execCommand(command); updateRichNote(editor); }
function makeCurrentTask(editor: HTMLElement) { const list = createTaskList(); const item = createTaskItem(); list.append(item); replaceRichBlock(editor, list); updateRichNote(editor); }
function renderEditor() {
  const note = selectedNote();
  if (!note) return `<main class="editor-panel empty-editor"><div class="app-actions"><span id="save-status"></span></div><div class="editor-empty"><span class="empty-icon"><i class="ph ph-note-pencil"></i></span><h2>No note selected</h2><p>Select a note, or start with a fresh page.</p><button class="primary-button" data-create-note>New note <kbd>${modLabel}+N</kbd></button></div></main>`;
  if (detachedNoteIds.has(note.id)) return `<main class="editor-panel detached-main-panel"><div class="app-actions"><span id="save-status" class="save-status ${savePhase}"></span><button class="icon-button" id="editor-menu" title="More note actions" aria-label="More note actions"><i class="ph ph-dots-three"></i></button></div><section class="detached-note-card" role="status" aria-live="polite"><div class="detached-window-mark"><i class="ph ph-browser"></i><span class="sync-dot"></span></div><span class="eyebrow">Live sync</span><h2>Editing in its own window</h2><p>Changes arrive here as you type. This page is paused so there is always one clear editor.</p><button class="primary-button attach-here" id="attach-here"><i class="ph ph-arrow-bend-up-left"></i><span>Attach here</span></button><p class="detached-hint"><i class="ph ph-cursor-click"></i>Select this note in the list to attach it here.</p></section></main>`;
  const canEdit = note.status === "active";
  return `<main class="editor-panel"><div class="app-actions"><span id="save-status" class="save-status ${savePhase}"></span><button class="action-button" id="focus-mode"><i class="ph ph-book-open-text"></i><span>${focusMode ? "Exit focus" : "Focus"}</span></button><span class="action-separator"></span>${note.status === "active" ? '<button class="icon-button" data-note-action="archive" title="Archive note"><i class="ph ph-archive-tray"></i></button>' : ""}<button class="icon-button" id="editor-menu" title="More note actions" aria-label="More note actions"><i class="ph ph-dots-three"></i></button></div>
    ${canEdit ? `<div class="format-bar" role="toolbar" aria-label="Formatting"><button data-block="H1" title="Heading 1">H₁</button><button data-block="H2" title="Heading 2">H₂</button><button data-block="H3" title="Heading 3">H₃</button><span></span><button data-rich-command="bold" title="Bold (${modLabel}+B)">B</button><button data-rich-command="italic" class="italic" title="Italic (${modLabel}+I)">I</button><button data-rich-command="strikeThrough" class="strike" title="Strike through">S</button><span></span><button class="icon-button" data-block="UL" title="Bulleted list"><i class="ph ph-list-bullets"></i></button><button class="icon-button" data-rich-task title="To-do list"><i class="ph ph-check-square"></i></button><span></span><button class="icon-button" data-rich-command="link" title="Add link"><i class="ph ph-link"></i></button><button class="icon-button" data-block="PRE" title="Code block"><i class="ph ph-code"></i></button><span></span><button class="icon-button" id="toolbar-more" title="Block menu"><i class="ph ph-dots-three"></i></button><button class="icon-button expand-editor" id="expand-editor" title="Focus mode"><i class="ph ph-arrows-out"></i></button></div>` : '<div class="readonly-bar"><i class="ph ph-info"></i>This note is read-only here. Restore it to edit.</div>'}
    <article class="editor-page"><div class="title-input" id="title-input" data-title-editor contenteditable="${canEdit}" role="textbox" aria-multiline="false" aria-label="Note title" spellcheck="true">${linkedPlainText(note.title) || "<br>"}</div><div class="note-meta"><span>${formatEditorDate(note.updated)}</span><span>·</span><span id="word-count">${wordCount(note.content)} words</span></div><div class="editor-wrap"><div id="markdown-editor" class="rich-editor" data-rich-editor contenteditable="${canEdit}" role="textbox" aria-multiline="true" aria-label="Note content" spellcheck="true">${markdownToRich(note.content)}</div>${canEdit ? slashMenuHtml() : ""}</div></article></main>`;
}
const slotHeight = 32;
const categoryFor = (todo: Todo) => state.todoCategories.find((category) => category.id === todo.categoryId) ?? state.todoCategories[0];
const dateKey = (date: Date) => `${date.getFullYear()}-${String(date.getMonth()+1).padStart(2,"0")}-${String(date.getDate()).padStart(2,"0")}`;
const dayStart = (offset: number) => { const date = new Date(plannerDate); date.setDate(date.getDate() + offset); return date; };
const localStart = (date: Date, minutes: number) => { const value = new Date(date); value.setMinutes(minutes,0,0); return value.toISOString(); };
const taskTime = (todo: Todo) => todo.scheduledStart ? new Date(todo.scheduledStart) : null;
function renderTasks() {
  const days = Number(state.plannerView); const active = state.todos.filter((todo) => !todo.completed); const rangeEnd = dayStart(days - 1);
  const title = days === 1 ? plannerDate.toLocaleDateString(undefined,{weekday:"long",month:"long",day:"numeric"}) : `${plannerDate.toLocaleDateString(undefined,{month:"short",day:"numeric"})} – ${rangeEnd.toLocaleDateString(undefined,{month:"short",day:"numeric",year:"numeric"})}`;
  return `<main class="planner-view" aria-label="Task planner"><aside class="planner-inbox"><header><div><span class="eyebrow">Daily workspace</span><h1>Tasks</h1></div><button class="icon-button" id="add-category" title="Add category" aria-label="Add category"><i class="ph ph-plus"></i></button></header><form id="quick-task-form" class="planner-quick-add"><i class="ph ph-plus"></i><input id="quick-task-input" autocomplete="off" placeholder="Add a task…" aria-label="New task"><kbd>Enter</kbd></form><div class="planner-inbox-meta"><span>${active.length} open</span><span>${state.todos.filter(t=>t.scheduledStart && !t.completed).length} scheduled</span></div><div class="planner-categories">${state.todoCategories.map(category => { const tasks = state.todos.filter(todo=>todo.categoryId===category.id); return `<section class="planner-category" data-category-id="${attr(category.id)}"><header><span class="category-dot" style="--category:${attr(category.color)}"></span><strong>${escapeHtml(category.name)}</strong><small>${tasks.filter(t=>!t.completed).length}</small><button data-category-add="${attr(category.id)}" aria-label="Add ${attr(category.name)} task"><i class="ph ph-plus"></i></button></header><div class="planner-task-list">${tasks.filter(todo=>!todo.completed).map(renderPlannerTodo).join("") || '<p class="planner-empty">No open tasks</p>'}</div></section>`; }).join("")}</div><section class="planner-complete"><button id="toggle-completed" aria-expanded="${!completedCollapsed}"><i class="ph ph-caret-${completedCollapsed?"right":"down"}"></i> Completed <small>${state.todos.filter(t=>t.completed).length}</small></button>${completedCollapsed?"":`<div>${state.todos.filter(t=>t.completed).map(renderPlannerTodo).join("")}</div>`}</section></aside><section class="planner-calendar"><header class="planner-toolbar"><div><button class="icon-button" data-planner-nav="prev" aria-label="Previous dates"><i class="ph ph-caret-left"></i></button><button class="today-button" data-planner-nav="today">Today</button><button class="icon-button" data-planner-nav="next" aria-label="Next dates"><i class="ph ph-caret-right"></i></button><h2>${title}</h2></div><div><input id="planner-date" type="date" value="${dateKey(plannerDate)}" aria-label="Jump to date"><select id="planner-view" aria-label="Calendar view">${[["1","1 day"],["3","3 days"],["4","4 days"],["7","Week"]].map(([value,label])=>`<option value="${value}" ${state.plannerView===value?"selected":""}>${label}</option>`).join("")}</select></div></header><div class="calendar-scroll" id="calendar-scroll"><div class="calendar-grid" style="--days:${days}"><div class="calendar-days"><div class="time-gutter"></div>${Array.from({length:days},(_,index)=>renderCalendarDayHeader(dayStart(index))).join("")}</div><div class="calendar-body"><div class="time-axis">${Array.from({length:24},(_,hour)=>`<span style="top:${hour*2*slotHeight}px">${String(hour).padStart(2,"0")}:00</span>`).join("")}</div><div class="calendar-columns">${Array.from({length:days},(_,index)=>renderCalendarColumn(dayStart(index))).join("")}</div></div></div></div></section>${taskMenuTodoId ? renderPlannerProperties(state.todos.find(t=>t.id===taskMenuTodoId)!) : ""}<div class="planner-live" aria-live="polite"></div></main>`;
}
function renderPlannerTodo(todo: Todo) { const category = categoryFor(todo); const time = taskTime(todo); return `<article class="planner-task-card ${todo.completed?"is-complete":""}" data-todo-id="${attr(todo.id)}" draggable="true" tabindex="0" role="button" aria-label="${attr(todo.text)}"><button class="task-check" data-toggle-todo="${attr(todo.id)}" aria-label="${todo.completed?"Reopen":"Complete"}"><i class="ph ph-check"></i></button><div><strong>${escapeHtml(todo.text)}</strong><small><span class="priority-dot ${todo.priority}"></span>${"•".repeat(todo.effort)}${"·".repeat(5-todo.effort)} ${time?` · ${time.toLocaleDateString(undefined,{month:"short",day:"numeric"})}`:" · Unscheduled"}</small></div><span class="task-chip" style="--category:${attr(todo.color||category.color)}">${escapeHtml(category.name)}</span><button class="task-more" data-todo-menu="${attr(todo.id)}" aria-label="Task properties"><i class="ph ph-dots-three"></i></button></article>`; }
function renderCalendarDayHeader(date: Date) { const today=dateKey(date)===dateKey(new Date()); return `<div class="calendar-day-header ${today?"is-today":""}"><span>${date.toLocaleDateString(undefined,{weekday:"short"})}</span><strong>${date.getDate()}</strong></div>`; }
function renderCalendarColumn(date: Date) { const key=dateKey(date); const nowDate=new Date(); const isToday=key===dateKey(nowDate); const tasks=state.todos.filter(todo=>todo.scheduledStart && dateKey(new Date(todo.scheduledStart))===key); const slots=Array.from({length:48},(_,i)=>`<div class="calendar-slot" data-slot-date="${key}" data-slot-minute="${i*30}"></div>`).join(""); const current=isToday?`<div class="now-line" style="top:${(nowDate.getHours()*60+nowDate.getMinutes())/30*slotHeight}px"><span>Now</span></div>`:""; return `<div class="calendar-column" data-calendar-date="${key}">${slots}${tasks.map(renderCalendarTask).join("")}${current}</div>`; }
function renderCalendarTask(todo: Todo) { const start=taskTime(todo)!; const category=categoryFor(todo); const minutes=start.getHours()*60+start.getMinutes(); const top=minutes/30*slotHeight; const height=Math.max(slotHeight,todo.durationMinutes/30*slotHeight); const ending=new Date(start.getTime()+todo.durationMinutes*60000); return `<article class="calendar-task ${todo.completed?"is-complete":""}" data-calendar-task="${attr(todo.id)}" data-todo-id="${attr(todo.id)}" tabindex="0" role="button" style="top:${top}px;height:${height}px;--task-color:${attr(todo.color||category.color)}"><div class="calendar-task-content"><span class="priority-band ${todo.priority}"></span><button class="calendar-check" data-toggle-todo="${attr(todo.id)}" aria-label="Complete task"><i class="ph ph-check"></i></button><div><small>${start.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})} – ${ending.toLocaleTimeString([],{hour:"numeric",minute:"2-digit"})}</small><strong>${escapeHtml(todo.text)}</strong></div></div><span class="calendar-resize" data-resize-todo="${attr(todo.id)}" title="Resize duration"></span></article>`; }
function renderPlannerProperties(todo: Todo) { const category=categoryFor(todo); const popover=plannerPopover ?? {x:window.innerWidth-315,y:90,returnId:todo.id}; return `<div class="planner-properties-backdrop" data-close-properties><section class="planner-properties" role="dialog" aria-modal="false" aria-label="Task properties" tabindex="-1" style="left:${popover.x}px;top:${popover.y}px"><header><div><span class="eyebrow">Task properties</span><h2>${escapeHtml(todo.text)}</h2></div><button class="icon-button" data-close-properties aria-label="Close"><i class="ph ph-x"></i></button></header><label>Category<select data-prop="categoryId">${state.todoCategories.map(c=>`<option value="${attr(c.id)}" ${c.id===todo.categoryId?"selected":""}>${escapeHtml(c.name)}</option>`).join("")}</select></label><div class="property-two"><label>Priority<select data-prop="priority">${["low","medium","high","urgent"].map(p=>`<option ${p===todo.priority?"selected":""}>${p}</option>`).join("")}</select></label><label>Effort<select data-prop="effort">${[1,2,3,4,5].map(n=>`<option value="${n}" ${n===todo.effort?"selected":""}>${n} / 5</option>`).join("")}</select></label></div><label>Color<input data-prop="color" type="color" value="${attr(todo.color||category.color)}"></label><label>Start<input data-prop="scheduledStart" type="datetime-local" value="${todo.scheduledStart?todo.scheduledStart.slice(0,16):""}"></label><label>Duration<select data-prop="durationMinutes">${[30,60,90,120,150,180,240].map(n=>`<option value="${n}" ${n===todo.durationMinutes?"selected":""}>${n} minutes</option>`).join("")}</select></label><footer><button class="secondary-button" data-unschedule>Unschedule</button><button class="secondary-button ${todo.completed?"":""}" data-toggle-todo="${attr(todo.id)}">${todo.completed?"Reopen":"Complete"}</button><button class="danger-button" data-delete-todo="${attr(todo.id)}">Delete</button></footer></section></div>`; }

function journalDateLabel(key: string) { const date = new Date(`${key}T12:00:00`); return date.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric", year: "numeric" }); }
function journalTime(iso: string) { return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", hour12: true }).replace(/\s([ap]m)$/i, (_match, marker: string) => ` ${marker.toUpperCase()}`); }
function journalSessions(entry: JournalEntry) { return (entry.content.match(/^##\s+.+$/gm) || []).length; }
function journalExcerpt(entry: JournalEntry) { return noteExcerpt(entry.content).slice(0, 170) || "An open page, waiting for a thought."; }
function entryForDate(key: string) { return state.journalEntries.find((entry) => entry.dateKey === key); }
function ensureJournalEntry(key: string) {
  let entry = entryForDate(key);
  if (!entry) { const timestamp = now(); entry = { id: uid("journal"), dateKey: key, content: "", created: timestamp, updated: timestamp }; state.journalEntries.push(entry); }
  selectedJournalDate = key; return entry;
}
function appendJournalSession(entry: JournalEntry) {
  const heading = `## ${journalTime(now())}`;
  entry.content = `${entry.content.trimEnd()}${entry.content.trim() ? "\n\n" : ""}${heading}\n\n`;
  entry.updated = now();
}
function openJournalToday(allowSession = true) {
  const key = dateKey(new Date()); const existing = entryForDate(key); const gap = existing ? Date.now() - new Date(existing.updated).getTime() : Infinity;
  const entry = ensureJournalEntry(key);
  if (allowSession && (!existing || gap > 4 * 60_000) && !journalOpening) appendJournalSession(entry);
  journalOpening = true; window.setTimeout(() => { journalOpening = false; }, 650);
  void saveState(false); renderApp();
  requestAnimationFrame(() => { const editor = document.querySelector<HTMLElement>("[data-journal-editor]"); if (editor) placeCaretEnd(editor.lastElementChild as HTMLElement ?? editor); });
}
function renderJournalCard(entry: JournalEntry) {
  const selected = entry.dateKey === selectedJournalDate; const today = entry.dateKey === dateKey(new Date()); const sessions = journalSessions(entry);
  return `<article class="journal-card ${selected ? "is-expanded" : ""} ${today ? "is-today" : ""} ${!entry.content.trim() ? "is-empty-day" : ""}" data-journal-card="${attr(entry.dateKey)}" aria-expanded="${selected}" tabindex="0"><header class="journal-card-header"><div><span class="journal-day">${escapeHtml(new Date(`${entry.dateKey}T12:00:00`).toLocaleDateString(undefined, { weekday: "short" }).toUpperCase())}</span><h2>${escapeHtml(journalDateLabel(entry.dateKey))}</h2></div><div class="journal-card-meta">${today ? '<span class="today-chip">Today</span>' : ""}<span><i class="ph ph-clock"></i>${sessions || "No"} ${sessions === 1 ? "session" : "sessions"}</span><time>${journalTime(entry.updated)}</time></div></header>${selected ? `<div class="journal-expanded"><div class="journal-session-rail"><span></span><small>Writing sessions are saved as you go</small></div><div class="journal-editor-wrap">${!entry.content.trim() ? '<p class="journal-begin">Begin this day with a small, honest note.</p>' : ""}<div class="rich-editor journal-rich-editor" data-rich-editor data-journal-editor contenteditable="true" role="textbox" aria-multiline="true" aria-label="Journal entry for ${attr(journalDateLabel(entry.dateKey))}" spellcheck="true">${markdownToRich(entry.content)}</div></div></div>` : `<button class="journal-card-preview" data-open-journal="${attr(entry.dateKey)}" aria-label="Open journal for ${attr(journalDateLabel(entry.dateKey))}"><p>${escapeHtml(journalExcerpt(entry))}</p><span>Open day <i class="ph ph-arrow-up-right"></i></span></button>`}</article>`;
}
function renderJournal() {
  const selected = selectedJournalDate || dateKey(new Date()); const entries = [...state.journalEntries].sort((a, b) => b.dateKey.localeCompare(a.dateKey));
  return `<main class="journal-view" aria-label="Journal"><header class="journal-header"><div><span class="eyebrow">Private practice</span><h1>Journal</h1><p>A quiet record of your day, one honest session at a time.</p></div><div class="journal-actions"><button class="secondary-button" id="journal-prev" aria-label="Previous day"><i class="ph ph-caret-left"></i></button><button class="today-button" id="journal-today">Today</button><button class="secondary-button" id="journal-next" aria-label="Next day"><i class="ph ph-caret-right"></i></button><input id="journal-date" type="date" value="${attr(selected)}" aria-label="Jump to journal date"><button class="primary-button" id="journal-new-time"><i class="ph ph-pen-nib"></i>New timestamp</button></div></header><div class="journal-status" aria-live="polite"></div><section class="journal-timeline" aria-label="Journal timeline">${entries.length ? entries.map(renderJournalCard).join("") : '<div class="journal-empty"><i class="ph ph-book-open-text"></i><h2>Your first page is waiting</h2><p>Open today to begin a private daily practice.</p></div>'}</section></main>`;
}
function renderSettings() {
  return `<main class="wide-view settings-view"><header class="wide-header"><div><span class="eyebrow">Preferences</span><h1>Settings</h1><p>Make Odo feel at home on this computer.</p></div><button class="icon-button close-wide" data-go-inbox title="Back to notes"><i class="ph ph-x"></i></button></header><div class="settings-sheet">
    <section class="settings-section"><div class="settings-copy"><i class="ph ph-database"></i><div><h2>Storage & backups</h2><p>${isDesktopApp ? "Your workspace is stored locally on this computer." : "Browser mode stores this workspace in local storage."}</p></div></div>${isDesktopApp ? `<div class="path-grid"><span>Database</span><code>${escapeHtml(storageInfo?.databasePath ?? "Loading…")}</code><span>Backups</span><code>${escapeHtml(storageInfo?.backupDirectory ?? "Loading…")}</code></div><button class="secondary-button" id="create-backup" ${storageError ? "disabled" : ""}><i class="ph ph-cloud-arrow-up"></i>Create backup</button>${storageError ? `<p class="inline-error">${escapeHtml(storageError)}</p>` : ""}` : '<div class="browser-note"><i class="ph ph-info"></i>Install and open the desktop app to create file backups.</div>'}</section>
    <section class="settings-section"><div class="settings-copy"><i class="ph ph-sparkle"></i><div><h2>Appearance & motion</h2><p>Keep transitions calm, quick, and comfortable.</p></div></div><label class="switch-row"><span>Interface motion<small>Menus, panels, and task feedback</small></span><input id="motion-toggle" type="checkbox" ${motionEnabled ? "checked" : ""}><span class="switch" aria-hidden="true"></span></label></section>
    <section class="settings-section"><div class="settings-copy"><i class="ph ph-keyboard"></i><div><h2>Keyboard shortcuts</h2><p>Everything important stays within reach.</p></div></div><div class="shortcut-grid">${[["New note",`${modLabel}+N`],["New folder",`${modLabel}+Shift+N`],["Search",`${modLabel}+K`],["Tasks",`${modLabel}+2`],["Next note",`${modLabel}+Tab`],["Previous note",`${modLabel}+Shift+Tab`],["Save",`${modLabel}+S`],["Focus mode",`${modLabel}+Shift+F`]].map(([label, key]) => `<span>${label}</span><kbd>${key}</kbd>`).join("")}</div><button class="secondary-button" id="show-help"><i class="ph ph-question"></i>View all shortcuts</button></section></div></main>`;
}
function renderDialogLayer() {
  return `<dialog id="folder-dialog" class="create-dialog"><form id="folder-form"><div class="dialog-icon"><i class="ph ph-folder-plus"></i></div><div><h2>Create a new folder</h2><p>Add it inside the current location.</p></div><label>Folder name<input id="folder-name" autocomplete="off" placeholder="e.g. Research" required></label><div class="dialog-actions"><button type="button" class="secondary-button" data-close-dialog>Cancel</button><button type="submit" class="primary-button">Create</button></div></form></dialog>
    <dialog id="move-dialog" class="create-dialog"><form id="move-form"><div class="dialog-icon"><i class="ph ph-folder-notch-open"></i></div><div><h2>Move note</h2><p>Choose its new home.</p></div><label>Folder<select id="move-folder">${state.folders.map((folder) => `<option value="${attr(folder.id)}">${escapeHtml(folder.name)}</option>`).join("")}</select></label><div class="dialog-actions"><button type="button" class="secondary-button" data-close-dialog>Cancel</button><button type="submit" class="primary-button">Move</button></div></form></dialog><div id="odo-dialog-layer">${renderOdoDialog()}</div>`;
}

function renderOdoDialog() {
  if (!dialogState) return "";
  const { options, error } = dialogState; const icon = options.destructive ? "ph-warning" : options.kind === "notice" ? "ph-info" : options.kind === "prompt" ? "ph-pencil-simple" : "ph-question";
  const prompt = options.kind === "prompt" ? `<label>${escapeHtml(options.label ?? "Name")}<input id="odo-dialog-input" value="${attr(options.initialValue ?? "")}" autocomplete="off" aria-describedby="odo-dialog-error"></label><p class="dialog-error" id="odo-dialog-error" aria-live="polite">${escapeHtml(error)}</p>` : options.path ? `<code class="dialog-path">${escapeHtml(options.path)}</code>` : "";
  const cancel = options.kind !== "notice" ? `<button type="button" class="secondary-button" data-dialog-cancel>${escapeHtml(options.cancelLabel ?? "No")}</button>` : "";
  return `<div class="odo-dialog-backdrop"><section class="odo-dialog ${options.destructive ? "is-destructive" : ""}" role="dialog" aria-modal="true" aria-labelledby="odo-dialog-title" aria-describedby="odo-dialog-message"><div class="dialog-icon"><i class="ph ${icon}"></i></div><div class="odo-dialog-copy"><h2 id="odo-dialog-title">${escapeHtml(options.title)}</h2><p id="odo-dialog-message">${escapeHtml(options.message)}</p></div>${prompt}<div class="dialog-actions">${cancel}<button type="button" class="${options.destructive ? "danger-button" : "primary-button"}" data-dialog-confirm>${escapeHtml(options.confirmLabel ?? (options.kind === "notice" ? "Done" : "Yes"))}</button></div></section></div>`;
}
function renderHelp() {
  if (!helpOpen) return "";
  const shortcuts = [[`${modLabel}+N`,"New note"],[`${modLabel}+Shift+N`,"New folder"],[`${modLabel}+K`,"Search"],[`${modLabel}+S`,"Save"],[`${modLabel}+Tab`,"Next note"],[`${modLabel}+Shift+Tab`,"Previous note"],[`${modLabel}+1`,"Inbox"],[`${modLabel}+2`,"Tasks"],[`${modLabel}+Shift+T`,"Tasks"],[`${modLabel}+D`,"Duplicate note"],[`${modLabel}+E`,"Archive note"],["F2","Rename selection"],["?","Shortcut help"],["Esc","Close / exit focus"]];
  return `<div class="overlay" id="help-overlay"><section class="help-panel" role="dialog" aria-modal="true" aria-labelledby="help-title"><header><div><span class="eyebrow">Keyboard first</span><h2 id="help-title">Shortcuts</h2></div><button class="icon-button" id="close-help" aria-label="Close shortcuts"><i class="ph ph-x"></i></button></header><div class="help-grid">${shortcuts.map(([key,label]) => `<span>${label}</span><kbd>${key}</kbd>`).join("")}</div><p>Menus also support arrow keys, Home, End, Enter, and Escape.</p></section></div>`;
}
function renderMenu() {
  if (!menuState) return "";
  return `<div class="context-menu" id="context-menu" role="menu" style="left:${menuState.x}px;top:${menuState.y}px">${menuState.items.map((item, index) => item.separator ? '<div class="menu-separator" role="separator"></div>' : `<button role="menuitem" data-menu-index="${index}" ${item.disabled ? "disabled" : ""} class="${item.danger ? "danger" : ""}"><i class="ph ${item.icon ?? "ph-dot"}"></i><span>${escapeHtml(item.label ?? "")}</span>${item.hint ? `<kbd>${escapeHtml(item.hint)}</kbd>` : ""}</button>`).join("")}</div>`;
}
function renderApp() {
  if (detachedNoteId) return;
  if (!workspaceReady) { document.querySelector<HTMLElement>("#app")!.innerHTML = '<main class="loading-shell" aria-label="Loading Odo"><span class="loading-wordmark">Odo</span><span class="loading-line"></span><span>Opening your workspace…</span></main>'; return; }
  repairState(); closeMenu(false);
  document.documentElement.classList.toggle("no-motion", !motionEnabled);
  const app = document.querySelector<HTMLElement>("#app")!;
  app.className = `${focusMode ? "focus-mode" : ""} ${sidebarCollapsed ? "sidebar-collapsed" : ""} view-${currentView}`;
  const content = currentView === "tasks" ? renderTasks() : currentView === "journal" ? renderJournal() : currentView === "settings" ? renderSettings() : renderNotesPanel() + renderEditor();
  app.innerHTML = `${renderSidebar()}${content}${renderDialogLayer()}${renderHelp()}<div id="menu-layer">${renderMenu()}</div><div id="drag-live" class="drag-live" role="status" aria-live="polite" aria-atomic="true"></div>`;
  updateSaveStatus(); bindEvents(); bindOdoDialog();
  if (currentView === "tasks") requestAnimationFrame(() => scrollPlannerToNow());
  if (newRowId) requestAnimationFrame(() => { document.querySelector(`[data-note-id="${CSS.escape(newRowId)}"]`)?.classList.remove("is-new"); newRowId = ""; });
}
window.setInterval(() => {
  if (currentView !== "tasks") return;
  const current = new Date();
  document.querySelectorAll<HTMLElement>(".now-line").forEach(line => line.style.top = `${(current.getHours()*60+current.getMinutes())/30*slotHeight}px`);
}, 60_000);

function refreshNoteList() {
  const list = document.querySelector<HTMLElement>("#note-list"); if (!list) return;
  const scroll = list.scrollTop; list.innerHTML = renderNoteRows(); list.scrollTop = scroll; bindNoteRows();
  const ordering = state.sortMode === "manual" ? "manual order" : state.sortMode === "newest" ? "newest first" : "oldest first";
  const count = document.querySelector<HTMLElement>("#note-count"); if (count) count.textContent = `${visibleNotes().length} notes · ${ordering}`;
}
function updateSelectedRowPreview() {
  const note = selectedNote(); if (!note) return;
  const row = document.querySelector<HTMLElement>(`[data-note-id="${CSS.escape(note.id)}"]`); if (!row) return;
  const title = row.querySelector<HTMLElement>(".note-title"); const excerpt = row.querySelector<HTMLElement>(".note-excerpt"); const time = row.querySelector<HTMLTimeElement>("time");
  if (title) title.innerHTML = linkedPlainText(note.title || "Untitled"); if (excerpt) excerpt.textContent = noteExcerpt(note.content); if (time) time.textContent = formatListDate(note.updated);
}
function reconcileNoteList() { refreshNoteList(); }
function focusTitle() { requestAnimationFrame(() => { const title = document.querySelector<HTMLElement>("#title-input"); if (!title) return; title.focus(); const range = document.createRange(); range.selectNodeContents(title); const selection = window.getSelection(); selection?.removeAllRanges(); selection?.addRange(range); }); }
function createNoteAndFocusTitle(folderId?: string) {
  const target = folderId && state.folders.some((folder) => folder.id === folderId) ? folderId : currentView === "notes" && state.folders.some((folder) => folder.id === state.selectedFolderId) ? state.selectedFolderId : "inbox";
  const note: Note = { id: uid("note"), folderId: target, title: "Untitled", content: "", updated: now(), status: "active", revision: 0 };
  state.notes.push(note); newRowId = note.id; state.selectedFolderId = target; state.selectedNoteId = note.id; currentView = "notes"; searchQuery = ""; slashOpen = false; void saveState(false); renderApp(); focusTitle();
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
  currentView = view; focusMode = false; searchQuery = ""; if (view === "journal") { openJournalToday(true); return; } repairState(); renderApp();
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

function showOdoDialog(options: DialogOptions): Promise<boolean | string | null> {
  return new Promise((resolve) => {
    dialogState = { options, trigger: document.activeElement as HTMLElement | null, resolve, error: "" };
    const layer = document.querySelector<HTMLElement>("#odo-dialog-layer"); if (layer) { layer.innerHTML = renderOdoDialog(); bindOdoDialog(); }
    requestAnimationFrame(() => (options.kind === "prompt" ? document.querySelector<HTMLInputElement>("#odo-dialog-input") : document.querySelector<HTMLButtonElement>(options.destructive ? "[data-dialog-cancel]" : "[data-dialog-confirm]"))?.focus());
  });
}
function confirmOdo(title: string, message: string, confirmLabel = "Yes", destructive = false) { return showOdoDialog({ kind: "confirm", title, message, confirmLabel, destructive }) as Promise<boolean>; }
function promptOdo(title: string, message: string, initialValue: string) { return showOdoDialog({ kind: "prompt", title, message, label: "Name", initialValue, confirmLabel: "Save", cancelLabel: "Cancel", validate: (value) => value.trim() ? null : "Enter a name to continue." }) as Promise<string | null>; }
function noticeOdo(title: string, message: string, path?: string) { return showOdoDialog({ kind: "notice", title, message, path, confirmLabel: "Done" }) as Promise<boolean>; }
function closeOdoDialog(value: boolean | string | null) {
  const current = dialogState; if (!current) return; dialogState = null; document.querySelector<HTMLElement>("#odo-dialog-layer")!.innerHTML = ""; current.resolve(value); requestAnimationFrame(() => current.trigger?.focus());
}
function bindOdoDialog() {
  const dialog = document.querySelector<HTMLElement>(".odo-dialog"); if (!dialog || !dialogState) return;
  const submit = () => { if (!dialogState) return; if (dialogState.options.kind === "prompt") { const input = dialog.querySelector<HTMLInputElement>("#odo-dialog-input")!; const error = dialogState.options.validate?.(input.value) ?? null; if (error) { dialogState.error = error; const errorNode = dialog.querySelector<HTMLElement>("#odo-dialog-error"); if (errorNode) errorNode.textContent = error; input.focus(); return; } closeOdoDialog(input.value.trim()); } else closeOdoDialog(true); };
  dialog.querySelector("[data-dialog-confirm]")?.addEventListener("click", submit); dialog.querySelector("[data-dialog-cancel]")?.addEventListener("click", () => closeOdoDialog(dialogState?.options.kind === "confirm" ? false : null));
  dialog.addEventListener("keydown", (event) => {
    event.stopPropagation(); const buttons = [...dialog.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]; const typing = isTypingTarget(event.target);
    if (event.key === "Escape") { event.preventDefault(); closeOdoDialog(dialogState?.options.kind === "confirm" ? false : null); return; }
    if (!typing && event.key.toLowerCase() === "y" && dialogState?.options.kind === "confirm") { event.preventDefault(); closeOdoDialog(true); return; }
    if (!typing && event.key.toLowerCase() === "n" && dialogState?.options.kind === "confirm") { event.preventDefault(); closeOdoDialog(false); return; }
    if (!typing && (event.key === "ArrowLeft" || event.key === "ArrowRight") && buttons.length > 1) { event.preventDefault(); const current = Math.max(0, buttons.indexOf(document.activeElement as HTMLButtonElement)); buttons[(current + (event.key === "ArrowRight" ? 1 : -1) + buttons.length) % buttons.length].focus(); return; }
    if (event.key === "Enter" && (dialogState?.options.kind === "prompt" || document.activeElement === dialog.querySelector("[data-dialog-confirm]"))) { event.preventDefault(); submit(); return; }
    if (event.key === "Tab") { const focusable = [...dialog.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), [tabindex]:not([tabindex="-1"])')]; if (!focusable.length) return; const first = focusable[0]; const last = focusable[focusable.length - 1]; if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); } else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); } }
  });
}

async function performMenuAction(action: string) {
  const [kind, verb, id] = action.split(":");
  if (kind === "list" && verb === "sort") {
    if (id === "newest" || id === "oldest" || id === "manual") state.sortMode = id;
    await saveState(false); renderApp(); return;
  }
  if (kind === "note") {
    const note = state.notes.find((item) => item.id === id); if (!note) return;
    if (verb === "rename") {
      if (note.status !== "active") { const name = await promptOdo("Rename note", `Choose a new name for “${note.title || "Untitled"}”.`, note.title || "Untitled"); if (!name) return; note.title = name; note.updated = now(); await saveState(false); renderApp(); return; }
      state.selectedNoteId = note.id; currentView = "notes"; state.selectedFolderId = note.folderId; renderApp(); focusTitle(); return;
    }
    if (verb === "pin") note.pinned = !note.pinned;
    if (verb === "duplicate") { const copy = { ...note, id: uid("note"), title: `${note.title || "Untitled"} copy`, updated: now(), status: "active" as NoteStatus }; state.notes.push(copy); state.selectedNoteId = copy.id; state.selectedFolderId = copy.folderId; currentView = "notes"; }
    if (verb === "move") { state.selectedNoteId = note.id; const dialog = document.querySelector<HTMLDialogElement>("#move-dialog")!; const select = document.querySelector<HTMLSelectElement>("#move-folder")!; select.value = note.folderId; dialog.showModal(); requestAnimationFrame(() => select.focus()); return; }
    if (verb === "archive") note.status = "archived";
    if (verb === "trash") note.status = "trash";
    if (verb === "restore") note.status = "active";
    if (verb === "delete" && await confirmOdo("Delete note permanently?", `“${note.title || "Untitled"}” will be removed forever. This cannot be undone.`, "Delete permanently", true)) state.notes = state.notes.filter((item) => item.id !== note.id); else if (verb === "delete") return;
    if (verb === "export") { await exportNote(note); return; }
    note.updated = now(); repairState(); await saveState(false); renderApp();
  }
  if (kind === "folder") {
    const folder = state.folders.find((item) => item.id === id); if (!folder) return;
    if (verb === "new-note") { createNoteAndFocusTitle(folder.id); return; }
    if (verb === "new-folder") { state.selectedFolderId = folder.id; currentView = "notes"; renderApp(); openFolderDialog(folder.id); return; }
    if (verb === "rename") { const name = await promptOdo("Rename folder", `Choose a new name for “${folder.name}”.`, folder.name); if (!name) return; folder.name = name; }
    if (verb === "delete") {
      if (folder.id === "inbox" || !await confirmOdo("Delete folder?", `“${folder.name}” and every subfolder will be deleted. Notes inside will move to Trash.`, "Delete folder", true)) return;
      const ids = descendants(folder.id); state.notes.filter((note) => ids.includes(note.folderId)).forEach((note) => { note.status = "trash"; note.updated = now(); }); state.folders = state.folders.filter((item) => !ids.includes(item.id)); state.selectedFolderId = "inbox";
    }
    repairState(); await saveState(false); renderApp();
  }
  if (kind === "todo") {
    const todo = state.todos.find((item) => item.id === id); if (!todo) return;
    if (verb === "toggle") todo.completed = !todo.completed;
    if (verb === "edit") { editingTodoId = todo.id; renderApp(); focusTodoEditor(todo.id); return; }
    if (verb === "duplicate") state.todos.push({ ...todo, id: uid("todo"), text: `${todo.text} copy`, created: now(), updated: now() });
    if (verb === "delete") { if (!await confirmOdo("Delete task?", `“${todo.text}” will be removed from your task list.`, "Delete task", true)) return; state.todos = state.todos.filter((item) => item.id !== todo.id); }
    todo.updated = now(); await saveState(false); renderApp();
  }
}
async function exportNote(note: Note) {
  if (isDesktopApp) { try { const path = await invoke<string>("export_note", { title: note.title || "Untitled", contents: note.content }); await noticeOdo("Note exported", `“${note.title || "Untitled"}” was exported as Markdown.`, path); } catch (error) { await noticeOdo("Export failed", `Could not export “${note.title || "Untitled"}”: ${String(error)}`); } return; }
  const blob = new Blob([`# ${note.title || "Untitled"}\n\n${note.content}`], { type: "text/markdown" }); const link = document.createElement("a"); link.href = URL.createObjectURL(blob); link.download = `${(note.title || "Untitled").replace(/[\\/:*?\"<>|]/g, "-")}.md`; link.click(); URL.revokeObjectURL(link.href);
}

function toggleTodo(id: string) { const todo = state.todos.find((item) => item.id === id); if (!todo) return; todo.completed = !todo.completed; todo.updated = now(); void saveState(false); renderApp(); }
function focusTodoEditor(id: string) { requestAnimationFrame(() => { const input = document.querySelector<HTMLInputElement>(`[data-edit-todo="${CSS.escape(id)}"]`); input?.focus(); input?.select(); }); }
function commitTodoEdit(input: HTMLInputElement) { const todo = state.todos.find((item) => item.id === input.dataset.editTodo); if (!todo) return; const text = input.value.trim(); if (text) { todo.text = text; todo.updated = now(); } editingTodoId = ""; void saveState(false); renderApp(); }
function updateSlashMenu() { const menu = document.querySelector<HTMLElement>("#slash-menu"); if (!menu) return; menu.classList.toggle("is-open", slashOpen); menu.querySelectorAll(".slash-command").forEach((item, index) => item.classList.toggle("is-active", index === slashIndex)); }
function insertCommand(index: number) { const editor = document.querySelector<HTMLElement>("[data-rich-editor]"); if (!editor) return; slashOpen = false; updateSlashMenu(); const value = commands[index].value; if (value === "## ") applyRichBlock("H2"); else if (value === "- [ ] ") makeCurrentTask(editor); else if (value === "- ") applyRichBlock("UL"); else if (value === "> ") applyRichBlock("BLOCKQUOTE"); else if (value.includes("```")) applyRichBlock("PRE"); else { const block = currentRichBlock(editor) ?? editor.appendChild(document.createElement("p")); block.textContent = ""; placeCaretEnd(block); } }
function updateRichNote(editor: HTMLElement) {
  const note = selectedNote(); if (!note || note.status !== "active") return;
  note.content = richToMarkdown(editor); note.updated = now(); const count = document.querySelector<HTMLElement>("#word-count"); if (count) count.textContent = `${wordCount(note.content)} words`; scheduleSave(); updateSelectedRowPreview();
  const block = currentRichBlock(editor); slashOpen = block?.tagName === "P" && /^\/[a-z-]*$/i.test(block.textContent ?? ""); if (slashOpen) slashIndex = 0; updateSlashMenu();
}
function normalizeRichBlocks(editor: HTMLElement) {
  // Chromium creates bare DIVs after Enter from headings and quotes. Make those
  // first-class paragraphs before interpreting the next Markdown shortcut.
  [...editor.children].filter((child) => child.tagName === "DIV").forEach((block) => {
    const selection = window.getSelection(); const caretWasHere = !!selection?.anchorNode && block.contains(selection.anchorNode);
    const paragraph = document.createElement("p"); while (block.firstChild) paragraph.append(block.firstChild); if (!paragraph.childNodes.length) paragraph.append(document.createElement("br")); block.replaceWith(paragraph); if (caretWasHere) placeCaretEnd(paragraph);
  });
}
function transformMarkdownShortcut(editor: HTMLElement) {
  normalizeRichBlocks(editor);
  const block = currentRichBlock(editor) ?? [...editor.children].find((child) => child.tagName === "P" && /^(#{1,3}\s|- |1\. |> |- \[ \] )$/.test((child.textContent ?? "").replace(/\u00a0/g, " "))) as HTMLElement | undefined;
  if (!block || block.tagName !== "P") return;
  const text = (block.textContent ?? "").replace(/\u00a0/g, " ");
  const heading = text.match(/^(#{1,3})\s$/); if (heading) { const next = document.createElement(`h${heading[1].length}`); next.append(document.createElement("br")); block.replaceWith(next); placeCaretEnd(next); return; }
  // Hold a bare "- " for one more character so the longer "- [ ] " task
  // shorthand wins. A normal bullet becomes a list as soon as its text starts.
  if (text === "- [ ] ") { makeCurrentTask(editor); return; }
  const bullet = text.match(/^-\s+(.+)$/); if (bullet && !text.startsWith("- [")) { const list = document.createElement("ul"); list.className = "rich-list"; const item = document.createElement("li"); item.textContent = bullet[1]; list.append(item); block.replaceWith(list); placeCaretEnd(item); return; }
  if (text === "1. ") { const list = document.createElement("ol"); list.className = "rich-list"; const item = document.createElement("li"); item.append(document.createElement("br")); list.append(item); block.replaceWith(list); placeCaretEnd(item); return; }
  if (text === "> ") { const quote = document.createElement("blockquote"); const paragraph = document.createElement("p"); paragraph.append(document.createElement("br")); quote.append(paragraph); block.replaceWith(quote); placeCaretEnd(paragraph); return; }
}
function toggleRichTask(input: HTMLInputElement, update = updateRichNote) { const item = input.closest<HTMLElement>(".rich-task"); item?.classList.toggle("is-checked", input.checked); const editor = input.closest<HTMLElement>("[data-rich-editor]"); if (editor) update(editor); }
function handleRichKeydown(event: KeyboardEvent, update = updateRichNote) {
  const editor = event.currentTarget as HTMLElement; const block = currentRichBlock(editor); const task = (event.target as HTMLElement).closest<HTMLElement>(".rich-task");
  if (slashOpen && ["ArrowDown","ArrowUp","Enter","Escape"].includes(event.key)) { event.preventDefault(); event.stopPropagation(); if (event.key === "ArrowDown") slashIndex = (slashIndex + 1) % commands.length; if (event.key === "ArrowUp") slashIndex = (slashIndex - 1 + commands.length) % commands.length; if (event.key === "Enter") { insertCommand(slashIndex); update(editor); return; } if (event.key === "Escape") slashOpen = false; updateSlashMenu(); return; }
  if (event.key === "Enter" && task) { event.preventDefault(); const label = task.querySelector<HTMLElement>(".rich-task-label"); if (!(label?.textContent ?? "").trim()) { const paragraph = document.createElement("p"); paragraph.append(document.createElement("br")); const list = task.parentElement!; if (list.children.length === 1) list.replaceWith(paragraph); else task.replaceWith(paragraph); placeCaretEnd(paragraph); } else { const next = createTaskItem(); task.after(next); placeCaretInTaskLabel(next.querySelector<HTMLElement>(".rich-task-label")!); } update(editor); return; }
  if (event.key === "Backspace" && block && ["H1","H2","H3","BLOCKQUOTE"].includes(block.tagName) && !(block.textContent ?? "").trim()) { event.preventDefault(); const paragraph = document.createElement("p"); paragraph.append(document.createElement("br")); block.replaceWith(paragraph); placeCaretEnd(paragraph); update(editor); return; }
  if (event.key === "Tab" && !slashOpen) { const list = (event.target as HTMLElement).closest("li"); if (list) { event.preventDefault(); document.execCommand(event.shiftKey ? "outdent" : "indent"); update(editor); } }
}
function bindRichEditor(editor: HTMLElement, update: (element: HTMLElement) => void) {
  editor.addEventListener("input", (event) => { transformMarkdownShortcut(editor); if (shouldRefreshAutoLinks(event as InputEvent)) refreshAutoLinks(editor); update(editor); }); editor.addEventListener("keydown", (event) => handleRichKeydown(event, update)); editor.addEventListener("change", (event) => { const checkbox = (event.target as HTMLElement).closest<HTMLInputElement>(".rich-task-check"); if (checkbox) toggleRichTask(checkbox, update); else update(editor); });
  editor.addEventListener("blur", () => { refreshAutoLinks(editor); update(editor); });
  editor.addEventListener("click", (event) => { const target = event.target as HTMLElement; const checkbox = target.closest<HTMLInputElement>(".rich-task-check"); if (checkbox) window.setTimeout(() => toggleRichTask(checkbox, update)); const anchor = target.closest<HTMLAnchorElement>("a[href]"); if (anchor) { event.preventDefault(); void openRichLink(anchor); } });
  editor.addEventListener("paste", (event) => { event.preventDefault(); const text = event.clipboardData?.getData("text/plain") ?? ""; document.execCommand("insertText", false, text); });
}

async function openDetachedNote(note: Note) {
  await saveState(false);
  if (isDesktopApp) { try { await invoke("open_note_window", { noteId: note.id, title: note.title || "Untitled" }); detachedNoteIds.add(note.id); renderApp(); } catch (error) { await noticeOdo("Could not open note window", String(error)); } return; }
  window.open(`${location.pathname}?note=${encodeURIComponent(note.id)}`, `odo-note-${note.id}`, "width=820,height=720");
}

function patchLiveNote(note: Note) {
  const index = state.notes.findIndex((item) => item.id === note.id);
  if (index < 0) return;
  state.notes[index] = { ...note, revision: note.revision ?? 0 };
  if (state.selectedNoteId === note.id && detachedNoteIds.has(note.id)) renderApp();
  else updateSelectedRowPreview();
}
async function attachDetachedNote(noteId: string) {
  if (!detachedNoteIds.has(noteId) || !isDesktopApp) return;
  try {
    const note = await invoke<Note | null>("attach_note_to_main", { noteId });
    if (note) patchLiveNote(note);
    detachedNoteIds.delete(noteId);
    state.selectedNoteId = noteId;
    renderApp();
    requestAnimationFrame(() => document.querySelector<HTMLElement>("#title-input")?.focus());
  } catch (error) { await noticeOdo("Could not attach note", String(error)); }
}

function announceDrag(message: string) {
  const live = document.querySelector<HTMLElement>("#drag-live");
  if (live) live.textContent = message;
}
function clearSortInsertion() {
  document.querySelectorAll<HTMLElement>(".is-sort-target").forEach((element) => { element.classList.remove("is-sort-target"); delete element.dataset.sortInsertion; });
  sortInsertionTargetId = "";
}
function clearDropTargetVisuals() { document.querySelectorAll<HTMLElement>(".is-drop-target").forEach((element) => element.classList.remove("is-drop-target")); }
function clearDragVisuals() {
  document.querySelectorAll<HTMLElement>(".is-dragging").forEach((element) => element.classList.remove("is-dragging"));
  clearDropTargetVisuals(); clearSortInsertion();
  dragImage?.remove(); dragImage = null;
  draggingNoteId = "";
}
function createDragImage(note: Note, event: DragEvent) {
  const card = document.createElement("div"); card.className = "note-drag-image"; card.setAttribute("aria-hidden", "true");
  const title = document.createElement("strong"); title.textContent = note.title || "Untitled";
  const excerpt = document.createElement("span"); excerpt.textContent = noteExcerpt(note.content);
  const mark = document.createElement("i"); mark.className = "ph ph-note-blank";
  card.append(mark, title, excerpt); document.body.append(card); dragImage = card;
  if (event.dataTransfer) event.dataTransfer.setDragImage(card, 22, 18);
}
function openFolderPath(folderId: string) {
  let cursor = state.folders.find((folder) => folder.id === folderId);
  while (cursor) {
    cursor.open = true;
    cursor = cursor.parentId ? state.folders.find((folder) => folder.id === cursor?.parentId) : undefined;
  }
}
function dropDestinationLabel(kind: string, id?: string) {
  if (kind === "folder") return state.folders.find((folder) => folder.id === id)?.name ?? "Inbox";
  return kind === "archive" ? "Archive" : "Trash";
}
async function moveDroppedNote(noteId: string, kind: "folder" | "archive" | "trash", folderId?: string) {
  // A detached window is the live editor. Close it only after its final SQLite
  // save has completed so a delayed child-window write can never undo this move.
  if (detachedNoteIds.has(noteId)) {
    announceDrag("Saving the detached note before moving it…");
    await attachDetachedNote(noteId);
    if (detachedNoteIds.has(noteId)) { announceDrag("The detached note could not be moved."); return; }
  }
  const note = state.notes.find((item) => item.id === noteId);
  if (!note) { announceDrag("That note is no longer available."); return; }
  const destination = dropDestinationLabel(kind, folderId);
  const unchanged = (kind === "folder" && note.folderId === folderId && note.status === "active") || (kind === "archive" && note.status === "archived") || (kind === "trash" && note.status === "trash");
  if (unchanged) { announceDrag(`“${note.title || "Untitled"}” is already in ${destination}.`); return; }

  if (kind === "folder") {
    const target = state.folders.find((folder) => folder.id === folderId);
    if (!target) { announceDrag("That folder is no longer available."); return; }
    note.folderId = target.id;
    note.status = "active";
    openFolderPath(target.id);
    state.selectedFolderId = target.id;
    currentView = "notes";
  } else if (kind === "archive") {
    note.status = "archived";
    currentView = "archived";
  } else {
    note.status = "trash";
    currentView = "trash";
  }
  note.updated = now();
  state.selectedNoteId = note.id;
  searchQuery = "";
  await saveState(false);
  renderApp();
  announceDrag(`Moved “${note.title || "Untitled"}” to ${destination}.`);
}

async function reorderDroppedNote(noteId: string, targetId: string, side: "before" | "after") {
  if (noteId === targetId) return;
  if (detachedNoteIds.has(noteId)) {
    announceDrag("Saving the detached note before arranging it…");
    await attachDetachedNote(noteId);
    if (detachedNoteIds.has(noteId)) { announceDrag("The detached note could not be arranged."); return; }
  }
  const orderedVisible = visibleNotes();
  const source = orderedVisible.find((note) => note.id === noteId);
  const sourceIndex = orderedVisible.findIndex((note) => note.id === noteId);
  const targetIndex = orderedVisible.findIndex((note) => note.id === targetId);
  if (!source || sourceIndex < 0 || targetIndex < 0) return;
  const desired = orderedVisible.filter((note) => note.id !== noteId);
  // The insertion marker is calculated in the rendered, pre-removal list.
  // When a note travels downward, removing it shifts the target back one slot.
  const targetIndexAfterRemoval = targetIndex - (sourceIndex < targetIndex ? 1 : 0);
  desired.splice(targetIndexAfterRemoval + (side === "after" ? 1 : 0), 0, source);
  if (desired.map((note) => note.id).join("|") === orderedVisible.map((note) => note.id).join("|")) return;

  // Keep all unrelated views in place. Only replace the slots occupied by the
  // currently visible note set, so manual order is stable across SQLite reloads.
  const visibleIds = new Set(orderedVisible.map((note) => note.id)); let next = 0;
  state.notes = state.notes.map((note) => visibleIds.has(note.id) ? desired[next++] : note);
  const moved = state.notes.find((note) => note.id === noteId); if (moved) moved.updated = now();
  state.sortMode = "manual";
  state.selectedNoteId = noteId;
  await saveState(false);
  renderApp();
  announceDrag(`Placed “${source.title || "Untitled"}” ${side} “${orderedVisible[targetIndex].title || "Untitled"}”. Manual order is on.`);
}

function bindDropTargets() {
  document.querySelectorAll<HTMLElement>("[data-drop-kind]").forEach((target) => {
    const acceptsDrag = (event: DragEvent) => !!draggingNoteId || !!event.dataTransfer?.types.includes("text/plain");
    target.addEventListener("dragenter", (event) => {
      if (!acceptsDrag(event)) return;
      event.preventDefault();
      clearSortInsertion(); clearDropTargetVisuals();
      target.classList.add("is-drop-target");
      const kind = target.dataset.dropKind!;
      announceDrag(`Move here: ${dropDestinationLabel(kind, target.dataset.dropId)}.`);
    });
    target.addEventListener("dragover", (event) => {
      if (!acceptsDrag(event)) return;
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      target.classList.add("is-drop-target");
    });
    target.addEventListener("dragleave", (event) => {
      if (!target.contains(event.relatedTarget as Node | null)) target.classList.remove("is-drop-target");
    });
    target.addEventListener("drop", (event) => {
      if (!acceptsDrag(event)) return;
      event.preventDefault();
      const noteId = draggingNoteId || event.dataTransfer?.getData("application/x-odo-note") || event.dataTransfer?.getData("text/plain");
      const kind = target.dataset.dropKind as "folder" | "archive" | "trash";
      const folderId = target.dataset.dropId;
      clearDragVisuals();
      if (noteId && kind) void moveDroppedNote(noteId, kind, folderId);
    });
  });
}

function bindNoteRows() {
  document.querySelectorAll<HTMLElement>("[data-note-id]").forEach((row) => {
    row.addEventListener("dragstart", (event) => {
      if ((event.target as HTMLElement).closest("button,a[href]")) { event.preventDefault(); return; }
      const id = row.dataset.noteId!;
      draggingNoteId = id;
      if (event.dataTransfer) { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("application/x-odo-note", id); event.dataTransfer.setData("text/plain", id); }
      row.classList.add("is-dragging");
      const note = state.notes.find((item) => item.id === id);
      if (note) createDragImage(note, event);
      announceDrag(`Moving “${note?.title || "Untitled"}”. Choose a folder, Archive, or Trash.`);
    });
    row.addEventListener("dragenter", (event) => {
      if (!draggingNoteId || draggingNoteId === row.dataset.noteId) return;
      event.preventDefault(); clearDropTargetVisuals();
    });
    row.addEventListener("dragover", (event) => {
      if (!draggingNoteId || draggingNoteId === row.dataset.noteId) return;
      event.preventDefault(); if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
      const rect = row.getBoundingClientRect(); const side = event.clientY < rect.top + rect.height / 2 ? "before" : "after";
      if (sortInsertionTargetId !== row.dataset.noteId || sortInsertionSide !== side) {
        clearSortInsertion(); sortInsertionTargetId = row.dataset.noteId!; sortInsertionSide = side;
        row.classList.add("is-sort-target"); row.dataset.sortInsertion = side;
        const note = state.notes.find((item) => item.id === row.dataset.noteId);
        announceDrag(`Place ${side} “${note?.title || "Untitled"}”.`);
      }
    });
    row.addEventListener("dragleave", (event) => { if (!row.contains(event.relatedTarget as Node | null) && sortInsertionTargetId === row.dataset.noteId) clearSortInsertion(); });
    row.addEventListener("drop", (event) => {
      if (!draggingNoteId || draggingNoteId === row.dataset.noteId) return;
      event.preventDefault(); const noteId = draggingNoteId; const targetId = row.dataset.noteId!; const side = sortInsertionSide;
      clearDragVisuals(); void reorderDroppedNote(noteId, targetId, side);
    });
    row.addEventListener("dragend", () => { suppressRowActivationUntil = Date.now() + 260; clearDragVisuals(); announceDrag(""); });
    row.addEventListener("click", (event) => { const target = event.target as HTMLElement; const anchor = target.closest<HTMLAnchorElement>("a[href]"); if (anchor) { event.preventDefault(); event.stopPropagation(); void openRichLink(anchor); return; } if (Date.now() < suppressRowActivationUntil || target.closest("[data-note-menu]")) return; const id = row.dataset.noteId!; state.selectedNoteId = id; if (detachedNoteIds.has(id)) void attachDetachedNote(id); else renderApp(); });
    row.addEventListener("dblclick", (event) => { if (Date.now() < suppressRowActivationUntil || (event.target as HTMLElement).closest("[data-note-menu],a[href]")) return; const note = state.notes.find((item) => item.id === row.dataset.noteId); if (note) void openDetachedNote(note); });
    row.addEventListener("contextmenu", (event) => { event.preventDefault(); const note = state.notes.find((item) => item.id === row.dataset.noteId); if (note) openMenu(noteMenuItems(note), row, event.clientX, event.clientY); });
    row.addEventListener("keydown", (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); const id = row.dataset.noteId!; state.selectedNoteId = id; if (detachedNoteIds.has(id)) void attachDetachedNote(id); else renderApp(); } if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); const rows = [...document.querySelectorAll<HTMLElement>("[data-note-id]")]; const index = rows.indexOf(row); const next = rows[index + (event.key === "ArrowDown" ? 1 : -1)]; if (next) { state.selectedNoteId = next.dataset.noteId!; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-note-id="${CSS.escape(state.selectedNoteId)}"]`)?.focus()); } } });
  });
  document.querySelectorAll<HTMLElement>("[data-note-menu]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); const note = state.notes.find((item) => item.id === button.dataset.noteMenu); if (note) openMenu(noteMenuItems(note), button); }));
}
function bindEvents() {
  bindNoteRows();
  bindDropTargets();
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
  document.querySelector("#list-menu")?.addEventListener("click", (event) => openMenu([
    { label: "Newest first", icon: "ph-clock-counter-clockwise", hint: state.sortMode === "newest" ? "Current" : undefined, action: "list:sort:newest" },
    { label: "Oldest first", icon: "ph-clock", hint: state.sortMode === "oldest" ? "Current" : undefined, action: "list:sort:oldest" },
    { label: "Manual order", icon: "ph-arrows-down-up", hint: state.sortMode === "manual" ? "Current" : "Drag notes to arrange" , action: "list:sort:manual" },
    { separator: true }, { label: "New note here", icon: "ph-note-pencil", hint: `${modLabel}+N`, action: `folder:new-note:${state.selectedFolderId}` }
  ], event.currentTarget as HTMLElement));
  document.querySelector("#editor-menu")?.addEventListener("click", (event) => { const note = selectedNote(); if (note) openMenu(noteMenuItems(note), event.currentTarget as HTMLElement); });
  document.querySelectorAll<HTMLElement>("[data-note-action]").forEach((button) => button.addEventListener("click", () => { const note = selectedNote(); if (note) void performMenuAction(`note:${button.dataset.noteAction}:${note.id}`); }));
  document.querySelector("#attach-here")?.addEventListener("click", () => { if (state.selectedNoteId) void attachDetachedNote(state.selectedNoteId); });
  const search = document.querySelector<HTMLInputElement>("#search-input"); search?.addEventListener("input", () => { searchQuery = search.value; repairState(); refreshNoteList(); });
  const title = document.querySelector<HTMLElement>("#title-input"); if (title) bindTitleEditor(title, (value) => { const note = selectedNote(); if (!note || note.status !== "active") return; note.title = value; note.updated = now(); scheduleSave(); updateSelectedRowPreview(); }, () => { const body = document.querySelector<HTMLElement>("[data-rich-editor]"); if (body) placeCaretEnd(body.lastElementChild as HTMLElement ?? body); }, () => document.querySelector<HTMLElement>(`[data-note-id="${CSS.escape(state.selectedNoteId)}"]`)?.focus());
  title?.addEventListener("blur", reconcileNoteList);
  const editor = document.querySelector<HTMLElement>("[data-rich-editor]"); if (editor) { bindRichEditor(editor, updateRichNote); editor.addEventListener("blur", reconcileNoteList); }
  document.querySelectorAll<HTMLElement>("[data-command-index]").forEach((button) => button.addEventListener("click", () => insertCommand(Number(button.dataset.commandIndex))));
  document.querySelectorAll<HTMLButtonElement>("[data-block]").forEach((button) => button.addEventListener("mousedown", (event) => event.preventDefault()));
  document.querySelectorAll<HTMLButtonElement>("[data-block]").forEach((button) => button.addEventListener("click", () => applyRichBlock(button.dataset.block as RichBlock)));
  document.querySelectorAll<HTMLButtonElement>("[data-rich-command]").forEach((button) => button.addEventListener("mousedown", (event) => event.preventDefault()));
  document.querySelectorAll<HTMLButtonElement>("[data-rich-command]").forEach((button) => button.addEventListener("click", () => applyRichInline(button.dataset.richCommand!)));
  document.querySelector<HTMLButtonElement>("[data-rich-task]")?.addEventListener("mousedown", (event) => event.preventDefault());
  document.querySelector<HTMLButtonElement>("[data-rich-task]")?.addEventListener("click", () => { const rich = document.querySelector<HTMLElement>("[data-rich-editor]"); if (rich) makeCurrentTask(rich); });
  const toggleFocus = () => { focusMode = !focusMode; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLElement>("[data-rich-editor]")?.focus()); };
  document.querySelector("#focus-mode")?.addEventListener("click", toggleFocus); document.querySelector("#expand-editor")?.addEventListener("click", toggleFocus);
  document.querySelector("#toolbar-more")?.addEventListener("click", () => { slashOpen = !slashOpen; slashIndex = 0; updateSlashMenu(); });
  bindTaskEvents(); bindJournalEvents(); bindSettingsEvents();
  document.querySelector("#close-help")?.addEventListener("click", () => { helpOpen = false; renderApp(); });
  document.querySelector("#help-overlay")?.addEventListener("click", (event) => { if (event.target === event.currentTarget) { helpOpen = false; renderApp(); } });
}
function bindJournalEvents() {
  const selectDay = (key: string, focus = false) => { ensureJournalEntry(key); void saveState(false); renderApp(); if (focus) requestAnimationFrame(() => document.querySelector<HTMLElement>("[data-journal-editor]")?.focus()); };
  document.querySelector("#journal-today")?.addEventListener("click", () => openJournalToday(true));
  document.querySelector("#journal-prev")?.addEventListener("click", () => { const date = new Date(`${(selectedJournalDate || dateKey(new Date()))}T12:00:00`); date.setDate(date.getDate() - 1); selectDay(dateKey(date)); });
  document.querySelector("#journal-next")?.addEventListener("click", () => { const date = new Date(`${(selectedJournalDate || dateKey(new Date()))}T12:00:00`); date.setDate(date.getDate() + 1); selectDay(dateKey(date)); });
  document.querySelector<HTMLInputElement>("#journal-date")?.addEventListener("change", (event) => { const key = (event.target as HTMLInputElement).value; if (key) selectDay(key); });
  document.querySelector("#journal-new-time")?.addEventListener("click", () => { const entry = ensureJournalEntry(selectedJournalDate || dateKey(new Date())); appendJournalSession(entry); void saveState(false); renderApp(); requestAnimationFrame(() => { const editor = document.querySelector<HTMLElement>("[data-journal-editor]"); if (editor) placeCaretEnd(editor.lastElementChild as HTMLElement ?? editor); }); });
  document.querySelectorAll<HTMLElement>("[data-journal-card]").forEach((card) => { const open = () => selectDay(card.dataset.journalCard!, false); card.addEventListener("click", (event) => { if ((event.target as HTMLElement).closest("[data-journal-editor],input,button")) return; if (card.dataset.journalCard !== selectedJournalDate) open(); }); card.addEventListener("keydown", (event) => { if (event.target !== card || (event.key !== "Enter" && event.key !== " ")) return; event.preventDefault(); open(); }); });
  document.querySelectorAll<HTMLElement>("[data-open-journal]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); selectDay(button.dataset.openJournal!, true); }));
  const editor = document.querySelector<HTMLElement>("[data-journal-editor]");
  if (editor) bindRichEditor(editor, (rich) => { const entry = entryForDate(selectedJournalDate); if (!entry) return; entry.content = richToMarkdown(rich); entry.updated = now(); clearTimeout(journalSaveTimer); journalSaveTimer = window.setTimeout(() => void saveState(false), 350); const status = document.querySelector<HTMLElement>(".journal-status"); if (status) status.textContent = `Saved ${journalTime(entry.updated)}`; });
}
function bindTaskEvents() {
  document.querySelector("#quick-task-form")?.addEventListener("submit", (event) => { event.preventDefault(); const input = document.querySelector<HTMLInputElement>("#quick-task-input")!; const text = input.value.trim(); if (!text) return; addPlannerTodo(text); });
  document.querySelector("#add-category")?.addEventListener("click", () => { void (showOdoDialog({ kind:"prompt", title:"New category", message:"Name a task category.", label:"Category name", initialValue:"", confirmLabel:"Create category", cancelLabel:"Cancel", validate:(value:string)=>value.trim()?null:"A category needs a name." }) as Promise<string | null>).then((value:string | null) => { if (typeof value !== "string") return; const colors=["#7b8e7c","#7499b1","#9184a8","#c5903f","#bd7064","#71808c"]; state.todoCategories.push({id:uid("category"),name:value.trim(),color:colors[state.todoCategories.length%colors.length],icon:"ph-tag"}); void saveState(false); renderApp(); }); });
  document.querySelectorAll<HTMLElement>("[data-category-add]").forEach(button => button.addEventListener("click", () => { const categoryId=button.dataset.categoryAdd!; const timestamp=now(); state.todos.unshift({id:uid("todo"),text:"Untitled task",completed:false,created:timestamp,updated:timestamp,categoryId,priority:"medium",effort:2,color:"",scheduledStart:null,durationMinutes:30}); void saveState(false); renderApp(); }));
  document.querySelectorAll<HTMLElement>("[data-planner-nav]").forEach(button => button.addEventListener("click", () => { const action=button.dataset.plannerNav; if(action==="today") plannerDate=new Date(); else plannerDate.setDate(plannerDate.getDate()+(action==="next"?Number(state.plannerView):-Number(state.plannerView))); plannerDate.setHours(0,0,0,0); renderApp(); scrollPlannerToNow(); }));
  document.querySelector<HTMLSelectElement>("#planner-view")?.addEventListener("change", (event)=>{ state.plannerView=(event.target as HTMLSelectElement).value as PlannerView; void saveState(false); renderApp(); });
  document.querySelector<HTMLInputElement>("#planner-date")?.addEventListener("change", (event)=>{ const value=(event.target as HTMLInputElement).value; if(value) { plannerDate=new Date(`${value}T00:00:00`); renderApp(); } });
  document.querySelectorAll<HTMLElement>("[data-toggle-todo]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); toggleTodo(button.dataset.toggleTodo!); }));
  document.querySelectorAll<HTMLElement>("[data-todo-id]").forEach((row) => { row.addEventListener("click", (event)=> { if(plannerPointerDragging || (event.target as HTMLElement).closest("button")) return; openPlannerProperties(row.dataset.todoId!, row); }); row.addEventListener("contextmenu", (event) => { event.preventDefault(); openPlannerProperties(row.dataset.todoId!, row); }); row.addEventListener("keydown", handleTaskKeydown); row.addEventListener("pointerdown", (event)=> { if(!(event.target as HTMLElement).closest("button")) startPlannerPointerDrag(event, row); }); row.addEventListener("dragstart", (event)=> { plannerDragTodoId=row.dataset.todoId!; event.dataTransfer?.setData("text/plain",plannerDragTodoId); if(event.dataTransfer) event.dataTransfer.effectAllowed="move"; row.classList.add("is-dragging"); }); row.addEventListener("dragend",()=>{plannerDragTodoId=""; document.querySelectorAll(".is-drop-target").forEach(node=>node.classList.remove("is-drop-target"));}); });
  document.querySelectorAll<HTMLElement>("[data-todo-menu]").forEach((button) => button.addEventListener("click", (event) => { event.stopPropagation(); openPlannerProperties(button.dataset.todoMenu!, button); }));
  document.querySelectorAll<HTMLElement>(".calendar-slot").forEach(slot=> { slot.addEventListener("dragover", event=>{ if(!plannerDragTodoId) return; event.preventDefault(); slot.classList.add("is-drop-target"); }); slot.addEventListener("dragleave",()=>slot.classList.remove("is-drop-target")); slot.addEventListener("drop",event=>{ event.preventDefault(); const todo=state.todos.find(t=>t.id===plannerDragTodoId); if(!todo) return; todo.scheduledStart=localStart(new Date(`${slot.dataset.slotDate}T00:00:00`),Number(slot.dataset.slotMinute)); todo.durationMinutes=todo.durationMinutes||30; todo.updated=now(); announcePlanner(`${todo.text} scheduled.`); plannerDragTodoId=""; void saveState(false); renderApp(); }); });
  document.querySelectorAll<HTMLElement>("[data-calendar-task]").forEach(block=> { block.addEventListener("dragstart",event=>{plannerDragTodoId=block.dataset.todoId!; event.dataTransfer?.setData("text/plain",plannerDragTodoId); block.classList.add("is-dragging");}); block.addEventListener("pointerdown",event=>{ if(!(event.target as HTMLElement).closest("button,[data-resize-todo]")) startPlannerPointerDrag(event,block); }); block.addEventListener("click",event=>{if(!plannerPointerDragging && !(event.target as HTMLElement).closest("button,[data-resize-todo]"))openPlannerProperties(block.dataset.todoId!,block);}); block.addEventListener("contextmenu",event=>{event.preventDefault();openPlannerProperties(block.dataset.todoId!,block);}); block.addEventListener("keydown",handleTaskKeydown); });
  document.querySelectorAll<HTMLElement>("[data-resize-todo]").forEach(handle=>handle.addEventListener("pointerdown", startResize));
  document.querySelectorAll<HTMLElement>("[data-close-properties]").forEach(node=>node.addEventListener("click",(event)=>{if(event.target===node || (event.target as HTMLElement).closest("[data-close-properties]")) closePlannerProperties();}));
  document.querySelectorAll<HTMLInputElement | HTMLSelectElement>("[data-prop]").forEach(input=>input.addEventListener("change",()=>{const todo=state.todos.find(t=>t.id===taskMenuTodoId); if(!todo) return; const field=input.dataset.prop as keyof Todo; let value:string|number|null=input.value; if(field==="effort"||field==="durationMinutes") value=Number(value); if(field==="scheduledStart") value=input.value ? new Date(input.value).toISOString() : null; (todo as unknown as Record<string,string|number|null>)[field]=value; todo.updated=now(); void saveState(false); renderApp();}));
  document.querySelector("[data-unschedule]")?.addEventListener("click",()=>{const todo=state.todos.find(t=>t.id===taskMenuTodoId);if(todo){todo.scheduledStart=null;todo.updated=now();taskMenuTodoId="";void saveState(false);renderApp();}});
  document.querySelectorAll<HTMLElement>("[data-delete-todo]").forEach(button=>button.addEventListener("click",()=>void deletePlannerTodo(button.dataset.deleteTodo!)));
  if (taskMenuTodoId) requestAnimationFrame(() => document.querySelector<HTMLElement>(".planner-properties")?.focus());
  document.querySelectorAll<HTMLInputElement>("[data-edit-todo]").forEach((input) => { input.addEventListener("keydown", (event) => { if (event.key === "Enter") commitTodoEdit(input); if (event.key === "Escape") { editingTodoId = ""; renderApp(); } }); input.addEventListener("blur", () => { if (editingTodoId) commitTodoEdit(input); }); });
  document.querySelector("#toggle-completed")?.addEventListener("click", () => { completedCollapsed = !completedCollapsed; renderApp(); });
  document.querySelector("#clear-completed")?.addEventListener("click", async () => { const count = state.todos.filter((todo) => todo.completed).length; if (await confirmOdo("Clear completed tasks?", `${count} completed ${count === 1 ? "task" : "tasks"} will be permanently removed.`, "Clear completed", true)) { state.todos = state.todos.filter((todo) => !todo.completed); void saveState(false); renderApp(); } });
}
function addPlannerTodo(text:string) { const timestamp=now(); state.todos.unshift({id:uid("todo"),text,completed:false,created:timestamp,updated:timestamp,categoryId:"inbox",priority:"medium",effort:2,color:"",scheduledStart:null,durationMinutes:30}); void saveState(false); renderApp(); requestAnimationFrame(()=>document.querySelector<HTMLInputElement>("#quick-task-input")?.focus()); }
function openPlannerProperties(id:string, trigger:HTMLElement) { const rect=trigger.getBoundingClientRect(); const width=285; const height=420; const x=Math.max(8,Math.min(rect.right+10,window.innerWidth-width-8)); const y=Math.max(8,Math.min(rect.top,window.innerHeight-height-8)); taskMenuTodoId=id; plannerPopover={x,y,returnId:id}; renderApp(); }
function closePlannerProperties() { const returnId=plannerPopover?.returnId; taskMenuTodoId=""; plannerPopover=null; renderApp(); if(returnId) requestAnimationFrame(()=>document.querySelector<HTMLElement>(`[data-todo-id="${CSS.escape(returnId)}"]`)?.focus()); }
function plannerDropAt(todoId:string, clientX:number, clientY:number) { const hit=document.elementFromPoint(clientX,clientY); const slot=hit?.closest<HTMLElement>("[data-slot-minute]"); const target=slot?.closest<HTMLElement>(".calendar-column") ?? hit?.closest<HTMLElement>(".calendar-column"); const todo=state.todos.find(item=>item.id===todoId); const exact=plannerPointerDrop; if((!target && !exact)||!todo) return false; // The captured slot is invariant even if a WebView retargets pointerup.
  const rect=target?.getBoundingClientRect(); const minute=exact?.minute ?? (slot ? Number(slot.dataset.slotMinute) : Math.max(0,Math.min(23*60+30,Math.round((clientY-(rect?.top ?? 0))/slotHeight)*30))); const date=exact?.date ?? target!.dataset.calendarDate!; todo.scheduledStart=localStart(new Date(`${date}T00:00:00`),minute); todo.durationMinutes=Math.max(30,todo.durationMinutes||30); todo.updated=now(); announcePlanner(`${todo.text} scheduled for ${new Date(todo.scheduledStart).toLocaleString()}.`); void saveState(false); renderApp(); return true; }
function startPlannerPointerDrag(event:PointerEvent, source:HTMLElement) { if(event.button!==0) return; const todoId=source.dataset.todoId; if(!todoId) return; const originX=event.clientX,originY=event.clientY; let started=false; let ghost:HTMLElement|null=null; plannerPointerDrop=null; const move=(moveEvent:PointerEvent)=>{ if(!started && Math.hypot(moveEvent.clientX-originX,moveEvent.clientY-originY)<6) return; if(!started){started=true;plannerPointerDragging=true;source.classList.add("is-dragging");ghost=document.createElement("div");ghost.className="planner-drag-ghost";ghost.textContent=state.todos.find(t=>t.id===todoId)?.text||"Task";document.body.append(ghost);} if(ghost){ghost.style.left=`${moveEvent.clientX+12}px`;ghost.style.top=`${moveEvent.clientY+12}px`;} document.querySelectorAll(".calendar-column.is-drop-target").forEach(node=>node.classList.remove("is-drop-target")); const hit=document.elementFromPoint(moveEvent.clientX,moveEvent.clientY); const slot=hit?.closest<HTMLElement>("[data-slot-minute]"); const column=slot?.closest<HTMLElement>(".calendar-column") ?? hit?.closest<HTMLElement>(".calendar-column"); plannerPointerDrop=slot&&column ? { date:column.dataset.calendarDate!, minute:Number(slot.dataset.slotMinute) } : null; column?.classList.add("is-drop-target");}; const up=(upEvent:PointerEvent)=>{window.removeEventListener("pointermove",move);window.removeEventListener("pointerup",up);source.classList.remove("is-dragging");ghost?.remove();document.querySelectorAll(".calendar-column.is-drop-target").forEach(node=>node.classList.remove("is-drop-target"));if(started) { plannerDropAt(todoId,upEvent.clientX,upEvent.clientY); window.setTimeout(()=>{plannerPointerDragging=false;plannerPointerDrop=null;},0); }}; window.addEventListener("pointermove",move);window.addEventListener("pointerup",up,{once:true}); }
function announcePlanner(message:string) { const live=document.querySelector<HTMLElement>(".planner-live"); if(live) live.textContent=message; }
function startResize(event:PointerEvent) { event.preventDefault(); event.stopPropagation(); const id=(event.currentTarget as HTMLElement).dataset.resizeTodo!; const todo=state.todos.find(t=>t.id===id); const block=(event.currentTarget as HTMLElement).closest<HTMLElement>("[data-calendar-task]"); if(!todo||!block) return; const startY=event.clientY; const origin=todo.durationMinutes; const move=(moveEvent:PointerEvent)=>{const next=Math.max(30,Math.min(24*60-(taskTime(todo)!.getHours()*60+taskTime(todo)!.getMinutes()),Math.round((origin+(moveEvent.clientY-startY)/slotHeight*30)/30)*30)); todo.durationMinutes=next; block.style.height=`${next/30*slotHeight}px`;}; const up=()=>{window.removeEventListener("pointermove",move);window.removeEventListener("pointerup",up);todo.updated=now();void saveState(false);renderApp();};window.addEventListener("pointermove",move);window.addEventListener("pointerup",up); }
async function deletePlannerTodo(id:string) { const todo=state.todos.find(t=>t.id===id); if(!todo || !await confirmOdo("Delete task?",`“${todo.text}” will be removed from your planner.`,"Delete task",true)) return; state.todos=state.todos.filter(t=>t.id!==id);taskMenuTodoId="";void saveState(false);renderApp(); }
function scrollPlannerToNow(){ const scroll=document.querySelector<HTMLElement>("#calendar-scroll"); if(scroll && dateKey(plannerDate)===dateKey(new Date())) scroll.scrollTop=Math.max(0,(new Date().getHours()*2-3)*slotHeight); }
function handleTaskKeydown(event: KeyboardEvent) {
  if ((event.target as HTMLElement).matches("input")) return; const row = event.currentTarget as HTMLElement; const id = row.dataset.todoId!; const rows = [...document.querySelectorAll<HTMLElement>("[data-todo-id]")]; const index = rows.indexOf(row);
  if (event.key === "ArrowDown" || event.key === "ArrowUp") { event.preventDefault(); rows[index + (event.key === "ArrowDown" ? 1 : -1)]?.focus(); }
  if (event.key === " ") { event.preventDefault(); toggleTodo(id); }
  if (event.key === "Enter") { event.preventDefault(); openPlannerProperties(id, row); }
  if (event.key === "Delete") { event.preventDefault(); const todo = state.todos.find((item) => item.id === id); if (todo) void (async () => { if (await confirmOdo("Delete task?", `“${todo.text}” will be removed from your task list.`, "Delete task", true)) { state.todos = state.todos.filter((item) => item.id !== id); await saveState(false); renderApp(); } })(); }
}
function bindSettingsEvents() {
  document.querySelector("#motion-toggle")?.addEventListener("change", (event) => { motionEnabled = (event.target as HTMLInputElement).checked; localStorage.setItem(MOTION_KEY, String(motionEnabled)); document.documentElement.classList.toggle("no-motion", !motionEnabled); });
  document.querySelector("#show-help")?.addEventListener("click", () => { helpOpen = true; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLButtonElement>("#close-help")?.focus()); });
  document.querySelector("#create-backup")?.addEventListener("click", async (event) => { const button = event.currentTarget as HTMLButtonElement; button.disabled = true; button.innerHTML = '<i class="ph ph-circle-notch"></i>Creating…'; try { const path = await invoke<string>("create_backup"); await noticeOdo("Backup created", "Your SQLite workspace backup is ready.", path); } catch (error) { await noticeOdo("Backup failed", `Could not create a backup: ${String(error)}`); } finally { renderApp(); } });
}

function isTypingTarget(target: EventTarget | null) { const element = target as HTMLElement | null; return !!element?.closest("input, textarea, select, [contenteditable=true]"); }
function cycleVisibleNote(direction: 1 | -1) {
  if (!["notes", "archived", "trash"].includes(currentView)) return;
  const rows = [...document.querySelectorAll<HTMLElement>("[data-note-id]")]; if (!rows.length) return;
  const active = document.activeElement; const region = active?.id === "title-input" ? "title" : active?.id === "markdown-editor" ? "body" : "row";
  const current = Math.max(0, rows.findIndex((row) => row.dataset.noteId === state.selectedNoteId)); const next = rows[(current + direction + rows.length) % rows.length];
  clearTimeout(saveTimer); void saveState(false); state.selectedNoteId = next.dataset.noteId!; renderApp();
  requestAnimationFrame(() => { if (region === "title") document.querySelector<HTMLElement>("#title-input")?.focus(); else if (region === "body") document.querySelector<HTMLElement>("[data-rich-editor]")?.focus(); else document.querySelector<HTMLElement>(`[data-note-id="${CSS.escape(state.selectedNoteId)}"]`)?.focus(); });
}
document.addEventListener("pointerdown", (event) => { if (menuState && !(event.target as HTMLElement).closest("#context-menu") && !(event.target as HTMLElement).closest("[data-note-menu],[data-folder-menu],[data-todo-menu],#editor-menu,#list-menu")) closeMenu(false); });
window.addEventListener("blur", () => closeMenu(false)); window.addEventListener("resize", () => closeMenu(false));
document.addEventListener("keydown", (event) => {
  if (event.isComposing || detachedNoteId) return;
  const command = event.metaKey || event.ctrlKey; const key = event.key.toLowerCase(); const typing = isTypingTarget(event.target);
  if (event.key === "Escape") { if (taskMenuTodoId) { event.preventDefault(); closePlannerProperties(); return; } if (menuState) { event.preventDefault(); closeMenu(); return; } const openDialog = document.querySelector<HTMLDialogElement>("dialog[open]"); if (openDialog) { event.preventDefault(); openDialog.close(); return; } if (helpOpen) { event.preventDefault(); helpOpen = false; renderApp(); return; } if (focusMode) { event.preventDefault(); focusMode = false; renderApp(); return; } }
  if (command && key === "n") { event.preventDefault(); if (event.shiftKey) openFolderDialog(); else createNoteAndFocusTitle(); return; }
  if (command && key === "k") { event.preventDefault(); if (!document.querySelector("#search-input")) { currentView = "notes"; renderApp(); } requestAnimationFrame(() => document.querySelector<HTMLInputElement>("#search-input")?.focus()); return; }
  if (command && key === "s") { event.preventDefault(); void saveState(); return; }
  if (command && event.key === "Tab") { event.preventDefault(); cycleVisibleNote(event.shiftKey ? -1 : 1); return; }
  if (command && event.shiftKey && key === "f") { event.preventDefault(); focusMode = !focusMode; if (currentView !== "notes") { currentView = "notes"; repairState(); } renderApp(); return; }
  if (command && ((event.shiftKey && key === "t") || key === "2")) { event.preventDefault(); setView("tasks"); return; }
  if (command && key === "1") { event.preventDefault(); state.selectedFolderId = "inbox"; setView("notes"); return; }
  if (!typing && command && key === "d") { const note = selectedNote(); if (note) { event.preventDefault(); void performMenuAction(`note:duplicate:${note.id}`); } return; }
  if (!typing && command && key === "e") { const note = selectedNote(); if (note?.status === "active") { event.preventDefault(); void performMenuAction(`note:archive:${note.id}`); } return; }
  if (!typing && event.key === "F2") { const note = selectedNote(); if (note && ["notes","archived","trash"].includes(currentView)) { event.preventDefault(); void performMenuAction(`note:rename:${note.id}`); } }
  if (!typing && event.key === "?") { event.preventDefault(); helpOpen = true; renderApp(); requestAnimationFrame(() => document.querySelector<HTMLButtonElement>("#close-help")?.focus()); }
});

async function renderDetachedEditor(noteId: string) {
  const app = document.querySelector<HTMLElement>("#app")!; app.className = "detached-app"; app.innerHTML = '<main class="detached-loading"><span class="loading-wordmark">Odo</span><span>Opening note…</span></main>';
  let note: Note | null = null;
  try { note = isDesktopApp ? await invoke<Note | null>("load_note", { noteId }) : state.notes.find((item) => item.id === noteId) ?? null; } catch (error) { app.innerHTML = `<main class="detached-error"><i class="ph ph-warning-circle"></i><h1>Could not open this note</h1><p>${escapeHtml(String(error))}</p></main>`; return; }
  if (!note) { app.innerHTML = '<main class="detached-error"><i class="ph ph-note-blank"></i><h1>Note not found</h1><p>It may have been deleted in another window.</p></main>'; return; }
  note = { ...note, revision: note.revision ?? 0 }; let saveChain: Promise<boolean> = Promise.resolve(true); let saveTimerId = 0; let phase: "saving" | "saved" | "error" = "saved";
  const shell = () => `<main class="detached-editor"><header class="detached-titlebar"><span class="detached-brand">Odo</span><span id="detached-save-state" class="detached-save-state ${phase}"><i class="ph ph-broadcast"></i>${phase === "saving" ? "Syncing…" : phase === "error" ? "Save failed" : "Live & saved"}</span><button class="attach-main-button" id="attach-main" aria-label="Attach this note to the main Odo window"><i class="ph ph-arrow-bend-up-left"></i>Attach to main window</button><kbd>${modLabel}+S</kbd></header><section class="detached-paper"><div id="detached-title" data-title-editor contenteditable="true" role="textbox" aria-multiline="false" aria-label="Note title" spellcheck="true">${linkedPlainText(note!.title) || "<br>"}</div><div id="detached-body" class="rich-editor detached-rich-editor" data-rich-editor contenteditable="true" role="textbox" aria-multiline="true" aria-label="Note content" spellcheck="true">${markdownToRich(note!.content)}</div></section><footer class="detached-status"><span id="detached-count">${wordCount(note!.content)} words</span><span><i class="ph ph-broadcast"></i> Live sync</span><span>${modLabel}+W closes</span></footer></main>`;
  const updatePhase = (next: typeof phase) => { phase = next; const node = document.querySelector<HTMLElement>("#detached-save-state"); if (node) { node.className = `detached-save-state ${phase}`; node.innerHTML = `<i class="ph ph-broadcast"></i>${phase === "saving" ? "Syncing…" : phase === "error" ? "Save failed" : "Live & saved"}`; } };
  const queueSave = (): Promise<boolean> => { if (!note) return Promise.resolve(false); clearTimeout(saveTimerId); updatePhase("saving"); saveChain = saveChain.catch(() => false).then(async () => { const snapshot = { ...note!, updated: now() }; note!.updated = snapshot.updated; try { if (isDesktopApp) note!.revision = await invoke<number>("save_note", { note: snapshot }); updatePhase("saved"); return true; } catch (error) { console.error("Could not save detached note:", error); updatePhase("error"); return false; } }); return saveChain; };
  const schedule = () => { clearTimeout(saveTimerId); updatePhase("saving"); saveTimerId = window.setTimeout(() => void queueSave(), 125); };
  const bindDetached = () => {
    const title = document.querySelector<HTMLElement>("#detached-title")!; const body = document.querySelector<HTMLElement>("#detached-body")!;
    bindTitleEditor(title, (value) => { note!.title = value; document.title = `${value || "Untitled"} — Odo`; schedule(); }, () => placeCaretEnd(body.lastElementChild as HTMLElement ?? body), () => body.focus());
    bindRichEditor(body, (editor) => { note!.content = richToMarkdown(editor); const count = document.querySelector<HTMLElement>("#detached-count"); if (count) count.textContent = `${wordCount(note!.content)} words`; schedule(); });
    body.addEventListener("keydown", (event) => { if (event.key === "Escape") { event.preventDefault(); title.focus(); } });
    title.addEventListener("blur", () => void queueSave()); body.addEventListener("blur", () => void queueSave());
    document.querySelector("#attach-main")?.addEventListener("click", async (event) => { const button = event.currentTarget as HTMLButtonElement; clearTimeout(saveTimerId); button.disabled = true; const saved = await queueSave(); if (!saved) { button.disabled = false; return; } try { if (isDesktopApp) await invoke("attach_note_to_main", { noteId }); window.close(); } catch (error) { console.error("Could not attach detached note:", error); updatePhase("error"); button.disabled = false; } });
  };
  document.title = `${note.title || "Untitled"} — Odo`; app.innerHTML = shell(); bindDetached();
  document.addEventListener("keydown", (event) => { if (event.isComposing) return; const command = event.metaKey || event.ctrlKey; if (command && event.key.toLowerCase() === "s") { event.preventDefault(); clearTimeout(saveTimerId); void queueSave(); } if (command && event.key.toLowerCase() === "w") { event.preventDefault(); clearTimeout(saveTimerId); void queueSave().then((saved) => { if (saved) window.close(); }); } });
  window.addEventListener("beforeunload", () => { clearTimeout(saveTimerId); void queueSave(); });
}

async function bootstrap() {
  if (detachedNoteId) { await renderDetachedEditor(detachedNoteId); return; }
  renderApp(); if (isDesktopApp) { await restoreDesktopWorkspace(); (await invoke<string[]>("list_detached_notes")).forEach((id) => detachedNoteIds.add(id)); renderApp(); await listen<Note>("note-updated", (event) => patchLiveNote(event.payload)); await listen<string>("note-detached", (event) => { detachedNoteIds.add(event.payload); if (state.selectedNoteId === event.payload) renderApp(); }); await listen<string>("note-attached", (event) => { detachedNoteIds.delete(event.payload); if (state.selectedNoteId === event.payload) renderApp(); }); }
}
void bootstrap();
