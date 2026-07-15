import "@phosphor-icons/web/regular/style.css";
import "./styles.css";

type NoteStatus = "active" | "archived" | "trash";
type Folder = {
  id: string;
  name: string;
  parentId: string | null;
  open: boolean;
  icon?: string;
};
type Note = {
  id: string;
  folderId: string;
  title: string;
  content: string;
  updated: string;
  status: NoteStatus;
  pinned?: boolean;
};
type Workspace = {
  folders: Folder[];
  notes: Note[];
  selectedFolderId: string;
  selectedNoteId: string;
};

const STORAGE_KEY = "odo-notes-workspace-v2";
const folders: Folder[] = [
  { id: "inbox", name: "Inbox", parentId: null, open: true, icon: "ph-tray" },
  {
    id: "work",
    name: "Work",
    parentId: null,
    open: true,
    icon: "ph-briefcase",
  },
  { id: "projects", name: "Projects", parentId: "work", open: true },
  { id: "summer", name: "Summer launch", parentId: "projects", open: true },
  { id: "brand", name: "Brand", parentId: "work", open: false },
  { id: "marketing", name: "Marketing", parentId: "work", open: false },
  { id: "notes", name: "Notes", parentId: null, open: false },
  { id: "meetings", name: "Meetings", parentId: null, open: false },
  { id: "reference", name: "Reference", parentId: null, open: false },
  {
    id: "personal",
    name: "Personal",
    parentId: null,
    open: true,
    icon: "ph-user-circle",
  },
  { id: "journal", name: "Journal", parentId: "personal", open: false },
  { id: "ideas", name: "Ideas", parentId: "personal", open: false },
  { id: "recipes", name: "Recipes", parentId: "personal", open: false },
  { id: "reading", name: "Reading", parentId: "personal", open: false },
];

const launchNote = `## Goals
- Drive awareness for the new release
- Convert interest into trial sign-ups
- Establish momentum for Q3 growth

## Key messages
- Built for focus and clarity
- Faster workflows, fewer clicks
- Designed with real feedback
- Security and privacy by default

## To-dos
- [x] Finalize messaging with team
- [ ] Review landing page copy
- [ ] Confirm influencer partnerships
- [ ] Prep launch day social posts

## Deliverables
- Launch landing page
- Product demo video
- Email campaign (3-part series)
- Social media toolkit

> Focus on clarity. Every asset should help the audience understand the value in seconds.

## Timeline
| Milestone | Date | Owner |
| --- | --- | --- |
| Messaging final | Jul 16 | Alex |
| Landing page live | Jul 21 | Sam |
| Campaign launch | Jul 28 | Team |

/`;

const notes: Note[] = [
  {
    id: "summer-launch",
    folderId: "summer",
    title: "Summer launch notes",
    updated: "2026-07-14T10:24:00",
    content: launchNote,
    status: "active",
    pinned: true,
  },
  {
    id: "campaign",
    folderId: "summer",
    title: "Campaign timeline",
    updated: "2026-07-13T16:10:00",
    content:
      "## Campaign timeline\n\n- Teaser campaign\n- Early access\n- Public launch\n- Customer stories",
    status: "active",
  },
  {
    id: "messaging",
    folderId: "summer",
    title: "Messaging framework",
    updated: "2026-07-12T13:45:00",
    content:
      "## Core narrative\n\nClarity should feel immediate. The product gets out of the way so people can do their best work.",
    status: "active",
  },
  {
    id: "updates",
    folderId: "summer",
    title: "Product updates",
    updated: "2026-07-10T09:15:00",
    content:
      "## July update\n\n- A faster command menu\n- Improved keyboard navigation\n- More comfortable reading width",
    status: "active",
  },
  {
    id: "checklist",
    folderId: "summer",
    title: "Launch checklist",
    updated: "2026-07-08T11:30:00",
    content:
      "## Before launch\n\n- [x] Confirm scope\n- [ ] Finish QA\n- [ ] Prepare release notes",
    status: "active",
  },
  {
    id: "stakeholders",
    folderId: "summer",
    title: "Stakeholder comms plan",
    updated: "2026-07-06T15:25:00",
    content:
      "## Audiences\n\n- Leadership\n- Product and design\n- Customer success\n- Launch partners",
    status: "active",
  },
  {
    id: "capture",
    folderId: "inbox",
    title: "Quick capture",
    updated: "2026-07-14T08:42:00",
    content:
      "Things to sort later.\n\n- Read the research summary\n- Follow up with Sam",
    status: "active",
  },
  {
    id: "weekly",
    folderId: "meetings",
    title: "Weekly product sync",
    updated: "2026-07-11T14:00:00",
    content: "## Agenda\n\n- Progress\n- Decisions\n- Blockers",
    status: "active",
  },
  {
    id: "book",
    folderId: "reading",
    title: "The Creative Act",
    updated: "2026-07-04T19:10:00",
    content:
      "## Notes\n\nA practice is something we return to, not something we finish.",
    status: "active",
  },
  {
    id: "archive-one",
    folderId: "notes",
    title: "Old launch outline",
    updated: "2026-06-02T12:00:00",
    content: "Archived outline",
    status: "archived",
  },
  {
    id: "trash-one",
    folderId: "inbox",
    title: "Untitled note",
    updated: "2026-07-01T12:00:00",
    content: "",
    status: "trash",
  },
];

function initialState(): Workspace {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) return JSON.parse(saved) as Workspace;
  } catch {
    localStorage.removeItem(STORAGE_KEY);
  }
  return {
    folders,
    notes,
    selectedFolderId: "summer",
    selectedNoteId: "summer-launch",
  };
}

let state = initialState();
let searchQuery = "";
let specialView: NoteStatus | null = null;
let sortNewest = true;
let focusMode = false;
let slashOpen = true;
let slashIndex = 0;
let dialogType: "note" | "folder" = "note";
let saveTimer = 0;

const commands = [
  {
    label: "Text",
    detail: "Just start writing.",
    icon: "ph-text-t",
    value: "",
  },
  {
    label: "Heading",
    detail: "Add a heading.",
    icon: "ph-text-h",
    value: "## ",
  },
  {
    label: "To-do list",
    detail: "Track tasks.",
    icon: "ph-check-square",
    value: "- [ ] ",
  },
  {
    label: "Bulleted list",
    detail: "Create a simple list.",
    icon: "ph-list-bullets",
    value: "- ",
  },
  {
    label: "Callout",
    detail: "Make something stand out.",
    icon: "ph-chat-centered-text",
    value: "> ",
  },
  {
    label: "Code",
    detail: "Write a code block.",
    icon: "ph-code",
    value: "```\n\n```",
  },
];

const escapeHtml = (value: string) =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\"/g, "&quot;")
    .replace(/'/g, "&#039;");
function saveState(show = true) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  const status = document.querySelector<HTMLElement>("#save-status");
  if (show && status)
    status.innerHTML = '<i class="ph ph-check-circle"></i> Saved';
}
function descendants(id: string): string[] {
  const children = state.folders.filter((f) => f.parentId === id);
  return [id, ...children.flatMap((child) => descendants(child.id))];
}
function folderCount(id: string) {
  const ids = descendants(id);
  return state.notes.filter(
    (note) => ids.includes(note.folderId) && note.status === "active",
  ).length;
}
function selectedNote() {
  return (
    state.notes.find((note) => note.id === state.selectedNoteId) ??
    state.notes[0]
  );
}
function currentFolder() {
  return (
    state.folders.find((folder) => folder.id === state.selectedFolderId) ??
    state.folders[0]
  );
}

function renderFolder(folder: Folder, depth = 0): string {
  const children = state.folders.filter(
    (candidate) => candidate.parentId === folder.id,
  );
  const selected = !specialView && state.selectedFolderId === folder.id;
  const toggle = children.length
    ? `<button class="folder-toggle" data-toggle-folder="${folder.id}" aria-label="Toggle ${escapeHtml(folder.name)}"><i class="ph ph-caret-${folder.open ? "down" : "right"}"></i></button>`
    : '<span class="folder-toggle-spacer"></span>';
  return `<div class="folder-branch"><div class="folder-row ${selected ? "is-selected" : ""}" data-folder-id="${folder.id}" style="--depth:${depth}" role="button" tabindex="0">${toggle}<i class="ph ${folder.icon ?? "ph-folder"} folder-icon"></i><span class="folder-name">${escapeHtml(folder.name)}</span><span class="folder-count">${folderCount(folder.id)}</span></div>${folder.open ? children.map((child) => renderFolder(child, depth + 1)).join("") : ""}</div>`;
}

function noteExcerpt(content: string) {
  return (
    content
      .replace(/[#>*`|\[\]-]/g, " ")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 76) || "Empty note"
  );
}
function formatListDate(iso: string) {
  const date = new Date(iso);
  const day = date.toISOString().slice(0, 10);
  if (day === "2026-07-14")
    return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (day === "2026-07-13") return "Yesterday";
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}
function visibleNotes() {
  const folderIds = specialView ? null : descendants(state.selectedFolderId);
  return state.notes
    .filter((note) => note.status === (specialView ?? "active"))
    .filter((note) => !folderIds || folderIds.includes(note.folderId))
    .filter((note) =>
      `${note.title} ${note.content}`
        .toLowerCase()
        .includes(searchQuery.toLowerCase()),
    )
    .sort(
      (a, b) =>
        (sortNewest ? 1 : -1) *
        (new Date(b.updated).getTime() - new Date(a.updated).getTime()),
    );
}
function viewTitle() {
  return specialView === "archived"
    ? "Archive"
    : specialView === "trash"
      ? "Trash"
      : currentFolder().name;
}

function renderNoteList() {
  const list = document.querySelector<HTMLElement>("#note-list");
  const count = document.querySelector<HTMLElement>("#note-count");
  const heading = document.querySelector<HTMLElement>("#note-list-title");
  if (!list || !count || !heading) return;
  const visible = visibleNotes();
  heading.firstChild!.textContent = viewTitle();
  count.textContent = `${visible.length} ${visible.length === 1 ? "note" : "notes"}`;
  list.innerHTML = visible.length
    ? visible
        .map(
          (note) =>
            `<button class="note-row ${note.id === state.selectedNoteId ? "is-selected" : ""}" data-note-id="${note.id}"><span class="note-heading"><span class="note-title-wrap">${note.pinned ? '<span class="pin-dot"></span>' : ""}<span class="note-title">${escapeHtml(note.title || "Untitled")}</span></span><time>${formatListDate(note.updated)}</time></span><span class="note-excerpt">${escapeHtml(noteExcerpt(note.content))}</span></button>`,
        )
        .join("")
    : '<div class="empty-notes"><i class="ph ph-note-blank"></i><strong>No notes here</strong><span>Create a note or try another search.</span></div>';
  list.querySelectorAll<HTMLElement>("[data-note-id]").forEach((row) =>
    row.addEventListener("click", () => {
      state.selectedNoteId = row.dataset.noteId!;
      saveState(false);
      renderApp();
    }),
  );
}

function slashMenuHtml() {
  return `<div class="slash-menu ${slashOpen ? "is-open" : ""}" id="slash-menu" role="listbox">${commands.map((command, index) => `<button class="slash-command ${index === slashIndex ? "is-active" : ""}" data-command-index="${index}"><span class="command-icon"><i class="ph ${command.icon}"></i></span><span><strong>${command.label}</strong><small>${command.detail}</small></span>${index === 0 ? "<kbd>↵</kbd>" : ""}</button>`).join("")}</div>`;
}

function renderApp() {
  const note = selectedNote();
  const app = document.querySelector<HTMLElement>("#app")!;
  app.className = focusMode ? "focus-mode" : "";
  app.innerHTML = `
    <aside class="folders-panel"><header class="brand-row"><button class="wordmark">Odo</button><button class="icon-button sidebar-toggle" title="Toggle sidebar"><i class="ph ph-sidebar-simple"></i></button></header><div class="panel-label-row"><span>Folders</span><button class="icon-button" id="new-folder" title="New folder"><i class="ph ph-plus"></i></button></div><nav class="folder-tree">${state.folders
      .filter((folder) => folder.parentId === null)
      .map((folder) => renderFolder(folder))
      .join(
        "",
      )}</nav><div class="library-links"><button class="library-link ${specialView === "archived" ? "is-selected" : ""}" data-special="archived"><i class="ph ph-archive-tray"></i><span>Archive</span><span>${state.notes.filter((note) => note.status === "archived").length}</span></button><button class="library-link ${specialView === "trash" ? "is-selected" : ""}" data-special="trash"><i class="ph ph-trash"></i><span>Trash</span><span>${state.notes.filter((note) => note.status === "trash").length}</span></button></div><button class="settings-link"><i class="ph ph-gear"></i><span>Settings</span></button></aside>
    <section class="notes-panel"><div class="global-bar"><label class="search-box"><i class="ph ph-magnifying-glass"></i><input id="search-input" type="search" placeholder="Search notes..." value="${escapeHtml(searchQuery)}"><kbd>⌘K</kbd></label><button class="new-note-button" id="new-note" title="New note"><i class="ph ph-note-pencil"></i></button></div><header class="notes-header"><button class="folder-heading" id="note-list-title">${escapeHtml(viewTitle())}<i class="ph ph-caret-down"></i></button><span id="note-count"></span><button class="sort-button" id="sort-notes" title="Reverse sort"><i class="ph ph-sort-ascending"></i></button></header><div class="note-list" id="note-list"></div></section>
    <main class="editor-panel"><div class="app-actions"><span id="save-status"><i class="ph ph-check-circle"></i> Saved</span><button class="action-button" id="focus-mode"><i class="ph ph-book-open-text"></i><span>${focusMode ? "Exit focus" : "Focus"}</span></button><span class="action-separator"></span><button class="icon-button" id="archive-note" title="Archive note"><i class="ph ph-archive-tray"></i></button><button class="icon-button" title="More actions"><i class="ph ph-dots-three"></i></button></div><div class="format-bar" role="toolbar"><button data-insert="## ">H₁</button><button data-insert="### ">H₂</button><button data-insert="#### ">H₃</button><span></span><button data-wrap="**">B</button><button data-wrap="_" class="italic">I</button><button data-wrap="~~" class="strike">S</button><span></span><button class="icon-button" data-insert="- " title="Bulleted list"><i class="ph ph-list-bullets"></i></button><button class="icon-button" data-insert="- [ ] " title="To-do list"><i class="ph ph-check-square"></i></button><span></span><button class="icon-button" data-wrap="[]()" data-link title="Add link"><i class="ph ph-link"></i></button><button class="icon-button" data-wrap="\`" title="Inline code"><i class="ph ph-code"></i></button><span></span><button class="icon-button" id="toolbar-more" title="Open block menu"><i class="ph ph-dots-three"></i></button><button class="icon-button expand-editor" id="expand-editor" title="Focus mode"><i class="ph ph-arrows-out"></i></button></div><article class="editor-page"><input class="title-input" id="title-input" value="${escapeHtml(note.title)}" aria-label="Note title"><div class="note-meta"><span>July 14, 2026 at 10:24 AM</span><span>·</span><span id="word-count">${note.content.trim().split(/\s+/).filter(Boolean).length} words</span></div><div class="editor-wrap"><textarea id="markdown-editor" aria-label="Markdown content" spellcheck="true">${escapeHtml(note.content)}</textarea>${slashMenuHtml()}</div></article></main>
    <dialog id="create-dialog" class="create-dialog"><form method="dialog"><div class="dialog-icon"><i class="ph ph-note-pencil"></i></div><div><h2 id="dialog-title">Create a new note</h2><p>Start with a fresh Markdown note.</p></div><label>Note name<input id="create-name" autocomplete="off" placeholder="Untitled note"></label><div class="dialog-actions"><button value="cancel" class="secondary-button">Cancel</button><button value="default" id="confirm-create" class="primary-button">Create</button></div></form></dialog>`;
  renderNoteList();
  bindEvents();
}

function showCreateDialog(type: "note" | "folder") {
  dialogType = type;
  const dialog = document.querySelector<HTMLDialogElement>("#create-dialog")!;
  dialog.querySelector<HTMLElement>("#dialog-title")!.textContent =
    `Create a new ${type}`;
  dialog.querySelector<HTMLElement>(".dialog-icon i")!.className =
    `ph ${type === "folder" ? "ph-folder-plus" : "ph-note-pencil"}`;
  dialog.querySelector<HTMLElement>("p")!.textContent =
    type === "folder"
      ? "Add a folder inside the current location."
      : "Start with a fresh Markdown note.";
  const label = dialog.querySelector<HTMLLabelElement>("label")!;
  label.childNodes[0].textContent = `${type === "folder" ? "Folder" : "Note"} name`;
  const input = dialog.querySelector<HTMLInputElement>("#create-name")!;
  input.placeholder = type === "folder" ? "e.g. Research" : "Untitled note";
  input.value = "";
  dialog.showModal();
  requestAnimationFrame(() => input.focus());
}

function createItem() {
  const input = document.querySelector<HTMLInputElement>("#create-name")!;
  const name =
    input.value.trim() ||
    (dialogType === "folder" ? "New folder" : "Untitled note");
  const id = `${dialogType}-${Date.now()}`;
  if (dialogType === "folder") {
    const selected = currentFolder();
    const noteId = `note-${Date.now()}`;
    state.folders.push({
      id,
      name,
      parentId: selected.id === "inbox" ? null : selected.id,
      open: true,
    });
    state.notes.push({
      id: noteId,
      folderId: id,
      title: "Untitled note",
      content: "",
      updated: new Date().toISOString(),
      status: "active",
    });
    selected.open = true;
    state.selectedFolderId = id;
    state.selectedNoteId = noteId;
    specialView = null;
    slashOpen = false;
  } else {
    const target = specialView ? "inbox" : state.selectedFolderId;
    state.notes.push({
      id,
      folderId: target,
      title: name,
      content: "",
      updated: new Date().toISOString(),
      status: "active",
    });
    state.selectedNoteId = id;
    state.selectedFolderId = target;
    specialView = null;
    slashOpen = false;
  }
  saveState(false);
  renderApp();
  if (dialogType === "note")
    document.querySelector<HTMLInputElement>("#title-input")?.select();
}

function updateSlashMenu() {
  const menu = document.querySelector<HTMLElement>("#slash-menu");
  if (!menu) return;
  menu.classList.toggle("is-open", slashOpen);
  menu
    .querySelectorAll(".slash-command")
    .forEach((item, index) =>
      item.classList.toggle("is-active", index === slashIndex),
    );
}
function insertCommand(index: number) {
  const editor =
    document.querySelector<HTMLTextAreaElement>("#markdown-editor")!;
  const cursor = editor.selectionStart;
  const lineStart = editor.value.lastIndexOf("\n", cursor - 1) + 1;
  const slash = editor.value.lastIndexOf("/", cursor);
  const start = slash >= lineStart ? slash : cursor;
  editor.setRangeText(commands[index].value, start, cursor, "end");
  if (commands[index].value.includes("\n\n"))
    editor.selectionStart = editor.selectionEnd = start + 4;
  slashOpen = false;
  updateSlashMenu();
  editor.focus();
  updateNote(editor);
}
function insertFormatting(value: string, wrap = false, link = false) {
  const editor =
    document.querySelector<HTMLTextAreaElement>("#markdown-editor")!;
  const start = editor.selectionStart;
  const end = editor.selectionEnd;
  const selected = editor.value.slice(start, end);
  const replacement = wrap
    ? link
      ? `[${selected || "link text"}](url)`
      : `${value}${selected}${value}`
    : value;
  editor.setRangeText(replacement, start, end, "end");
  editor.focus();
  updateNote(editor);
}

function updateNote(editor: HTMLTextAreaElement) {
  const note = selectedNote();
  const normalized = editor.value.replace(
    /^(\s*)-\s*\[\](?=\s|$)/gm,
    "$1- [ ]",
  );
  if (normalized !== editor.value) {
    const cursor =
      editor.selectionStart + normalized.length - editor.value.length;
    editor.value = normalized;
    editor.selectionStart = editor.selectionEnd = cursor;
  }
  note.content = editor.value;
  note.updated = new Date().toISOString();
  document.querySelector<HTMLElement>("#word-count")!.textContent =
    `${note.content.trim().split(/\s+/).filter(Boolean).length} words`;
  document.querySelector<HTMLElement>("#save-status")!.innerHTML =
    '<i class="ph ph-circle-notch"></i> Saving';
  clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => saveState(), 350);
  const cursor = editor.selectionStart;
  const lineStart = editor.value.lastIndexOf("\n", cursor - 1) + 1;
  slashOpen = /^\/[a-z-]*$/i.test(editor.value.slice(lineStart, cursor));
  if (slashOpen) slashIndex = 0;
  updateSlashMenu();
  renderNoteList();
}

function bindEvents() {
  document.querySelectorAll<HTMLElement>("[data-folder-id]").forEach((row) => {
    row.addEventListener("click", (event) => {
      if ((event.target as HTMLElement).closest("[data-toggle-folder]")) return;
      specialView = null;
      state.selectedFolderId = row.dataset.folderId!;
      const first = state.notes.find(
        (note) =>
          descendants(state.selectedFolderId).includes(note.folderId) &&
          note.status === "active",
      );
      if (first) state.selectedNoteId = first.id;
      saveState(false);
      renderApp();
    });
    row.addEventListener("dblclick", () => {
      const folder = state.folders.find(
        (item) => item.id === row.dataset.folderId,
      );
      const name =
        folder && window.prompt("Rename folder", folder.name)?.trim();
      if (folder && name) {
        folder.name = name;
        saveState(false);
        renderApp();
      }
    });
  });
  document
    .querySelectorAll<HTMLElement>("[data-toggle-folder]")
    .forEach((toggle) =>
      toggle.addEventListener("click", (event) => {
        event.stopPropagation();
        const folder = state.folders.find(
          (item) => item.id === toggle.dataset.toggleFolder,
        );
        if (folder) {
          folder.open = !folder.open;
          saveState(false);
          renderApp();
        }
      }),
    );
  document.querySelectorAll<HTMLElement>("[data-special]").forEach((button) =>
    button.addEventListener("click", () => {
      specialView = button.dataset.special as NoteStatus;
      const first = visibleNotes()[0];
      if (first) state.selectedNoteId = first.id;
      renderApp();
    }),
  );
  document
    .querySelector("#new-folder")
    ?.addEventListener("click", () => showCreateDialog("folder"));
  document
    .querySelector("#new-note")
    ?.addEventListener("click", () => showCreateDialog("note"));
  document
    .querySelector("#confirm-create")
    ?.addEventListener("click", (event) => {
      event.preventDefault();
      createItem();
      document.querySelector<HTMLDialogElement>("#create-dialog")?.close();
    });
  const search = document.querySelector<HTMLInputElement>("#search-input")!;
  search.addEventListener("input", () => {
    searchQuery = search.value;
    renderNoteList();
  });
  document.querySelector("#sort-notes")?.addEventListener("click", () => {
    sortNewest = !sortNewest;
    renderNoteList();
  });
  const title = document.querySelector<HTMLInputElement>("#title-input")!;
  title.addEventListener("input", () => {
    const note = selectedNote();
    note.title = title.value;
    note.updated = new Date().toISOString();
    clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => saveState(), 350);
    renderNoteList();
  });
  const editor =
    document.querySelector<HTMLTextAreaElement>("#markdown-editor")!;
  editor.addEventListener("input", () => updateNote(editor));
  editor.addEventListener("keydown", (event) => {
    if (slashOpen) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        slashIndex = (slashIndex + 1) % commands.length;
        updateSlashMenu();
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        slashIndex = (slashIndex - 1 + commands.length) % commands.length;
        updateSlashMenu();
      }
      if (event.key === "Enter") {
        event.preventDefault();
        insertCommand(slashIndex);
      }
      if (event.key === "Escape") {
        event.preventDefault();
        slashOpen = false;
        updateSlashMenu();
      }
    }
    if (event.key === "Tab" && !slashOpen) {
      event.preventDefault();
      editor.setRangeText(
        "  ",
        editor.selectionStart,
        editor.selectionEnd,
        "end",
      );
      updateNote(editor);
    }
  });
  document
    .querySelectorAll<HTMLElement>("[data-command-index]")
    .forEach((button) =>
      button.addEventListener("click", () =>
        insertCommand(Number(button.dataset.commandIndex)),
      ),
    );
  document
    .querySelectorAll<HTMLButtonElement>("[data-insert]")
    .forEach((button) =>
      button.addEventListener("click", () =>
        insertFormatting(button.dataset.insert!),
      ),
    );
  document
    .querySelectorAll<HTMLButtonElement>("[data-wrap]")
    .forEach((button) =>
      button.addEventListener("click", () =>
        insertFormatting(
          button.dataset.wrap!,
          true,
          button.hasAttribute("data-link"),
        ),
      ),
    );
  const toggleFocus = () => {
    focusMode = !focusMode;
    renderApp();
    requestAnimationFrame(() =>
      document.querySelector<HTMLTextAreaElement>("#markdown-editor")?.focus(),
    );
  };
  document.querySelector("#focus-mode")?.addEventListener("click", toggleFocus);
  document
    .querySelector("#expand-editor")
    ?.addEventListener("click", toggleFocus);
  document
    .querySelector(".sidebar-toggle")
    ?.addEventListener("click", toggleFocus);
  document.querySelector("#toolbar-more")?.addEventListener("click", () => {
    slashOpen = !slashOpen;
    slashIndex = 0;
    updateSlashMenu();
  });
  document.querySelector("#archive-note")?.addEventListener("click", () => {
    const note = selectedNote();
    note.status = note.status === "archived" ? "active" : "archived";
    specialView = note.status === "archived" ? "archived" : null;
    saveState(false);
    renderApp();
  });
}

document.addEventListener("keydown", (event) => {
  const command = event.metaKey || event.ctrlKey;
  if (command && event.key.toLowerCase() === "k") {
    event.preventDefault();
    document.querySelector<HTMLInputElement>("#search-input")?.focus();
  }
  if (command && event.key.toLowerCase() === "n" && !event.shiftKey) {
    event.preventDefault();
    showCreateDialog("note");
  }
  if (command && event.shiftKey && event.key.toLowerCase() === "n") {
    event.preventDefault();
    showCreateDialog("folder");
  }
  if (command && event.shiftKey && event.key.toLowerCase() === "f") {
    event.preventDefault();
    focusMode = !focusMode;
    renderApp();
  }
  if (command && event.key.toLowerCase() === "s") {
    event.preventDefault();
    saveState();
  }
});

renderApp();
