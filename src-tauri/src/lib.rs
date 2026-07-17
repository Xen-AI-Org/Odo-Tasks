use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tauri_plugin_autostart::ManagerExt;

pub mod mcp;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Folder {
    id: String,
    name: String,
    parent_id: Option<String>,
    open: bool,
    icon: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Note {
    id: String,
    folder_id: String,
    title: String,
    content: String,
    updated: String,
    status: String,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    revision: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Todo {
    id: String,
    text: String,
    #[serde(default)]
    completed: bool,
    created: String,
    updated: String,
    #[serde(default = "default_category_id")]
    category_id: String,
    #[serde(default = "default_priority")]
    priority: String,
    #[serde(default = "default_effort")]
    effort: i64,
    #[serde(default)]
    color: String,
    #[serde(default)]
    scheduled_start: Option<String>,
    #[serde(default = "default_duration")]
    duration_minutes: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodoCategory {
    id: String,
    name: String,
    color: String,
    #[serde(default)]
    icon: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalEntry {
    id: String,
    date_key: String,
    content: String,
    created: String,
    updated: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Workspace {
    folders: Vec<Folder>,
    notes: Vec<Note>,
    #[serde(default)]
    todos: Vec<Todo>,
    #[serde(default)]
    todo_categories: Vec<TodoCategory>,
    #[serde(default)]
    journal_entries: Vec<JournalEntry>,
    selected_folder_id: String,
    selected_note_id: String,
    #[serde(default = "default_sort_mode")]
    sort_mode: String,
    #[serde(default = "default_planner_view")]
    planner_view: String,
}

fn default_sort_mode() -> String {
    "newest".into()
}
fn default_category_id() -> String {
    "inbox".into()
}
fn default_priority() -> String {
    "medium".into()
}
fn default_effort() -> i64 {
    2
}
fn default_duration() -> i64 {
    30
}
fn default_planner_view() -> String {
    "3".into()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageInfo {
    database_path: String,
    backup_directory: String,
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not locate the app-data directory: {error}"))?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the app-data directory: {error}"))?;
    Ok(directory)
}

fn database_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("workspace.sqlite3"))
}

fn backup_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app_data_dir(app)?.join("backups");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the backup directory: {error}"))?;
    Ok(directory)
}

fn open_database(app: &AppHandle) -> Result<Connection, String> {
    let connection = Connection::open(database_path(app)?)
        .map_err(|error| format!("Could not open the workspace database: {error}"))?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("Could not configure the workspace database: {error}"))?;
    initialize_database(&connection)?;
    Ok(connection)
}

pub fn initialize_database(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS folders (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               parent_id TEXT,
               is_open INTEGER NOT NULL,
               icon TEXT,
               position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS notes (
               id TEXT PRIMARY KEY,
               folder_id TEXT NOT NULL,
               title TEXT NOT NULL,
               content TEXT NOT NULL,
               updated TEXT NOT NULL,
               status TEXT NOT NULL,
               pinned INTEGER NOT NULL DEFAULT 0,
               position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS todos (
               id TEXT PRIMARY KEY,
               text TEXT NOT NULL,
               completed INTEGER NOT NULL DEFAULT 0,
               created TEXT NOT NULL,
               updated TEXT NOT NULL,
               position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS app_state (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS todo_categories (
               id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL, icon TEXT NOT NULL DEFAULT '', position INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS journal_entries (
               id TEXT PRIMARY KEY, date_key TEXT NOT NULL UNIQUE, content TEXT NOT NULL DEFAULT '', created TEXT NOT NULL, updated TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0
             );",
        )
        .map_err(|error| format!("Could not initialize the workspace database: {error}"))?;

    let has_revision = connection
        .prepare("PRAGMA table_info(notes)")
        .and_then(|mut statement| {
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(columns.iter().any(|column| column == "revision"))
        })
        .map_err(|error| format!("Could not inspect the notes schema: {error}"))?;
    if !has_revision {
        connection
            .execute(
                "ALTER TABLE notes ADD COLUMN revision INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(|error| format!("Could not upgrade the notes schema: {error}"))?;
    }
    let todo_columns = connection
        .prepare("PRAGMA table_info(todos)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| format!("Could not inspect tasks schema: {error}"))?;
    for (column, ddl) in [
        (
            "category_id",
            "ALTER TABLE todos ADD COLUMN category_id TEXT NOT NULL DEFAULT 'inbox'",
        ),
        (
            "priority",
            "ALTER TABLE todos ADD COLUMN priority TEXT NOT NULL DEFAULT 'medium'",
        ),
        (
            "effort",
            "ALTER TABLE todos ADD COLUMN effort INTEGER NOT NULL DEFAULT 2",
        ),
        (
            "color",
            "ALTER TABLE todos ADD COLUMN color TEXT NOT NULL DEFAULT ''",
        ),
        (
            "scheduled_start",
            "ALTER TABLE todos ADD COLUMN scheduled_start TEXT",
        ),
        (
            "duration_minutes",
            "ALTER TABLE todos ADD COLUMN duration_minutes INTEGER NOT NULL DEFAULT 30",
        ),
    ] {
        if !todo_columns.iter().any(|item| item == column) {
            connection
                .execute(ddl, [])
                .map_err(|error| format!("Could not upgrade tasks schema: {error}"))?;
        }
    }
    connection.execute("INSERT OR IGNORE INTO todo_categories (id,name,color,icon,position) VALUES ('inbox','Inbox','#7b8e7c','ph-tray',0)", []).map_err(|error| format!("Could not seed task categories: {error}"))?;
    Ok(())
}

fn write_workspace(connection: &mut Connection, contents: &str) -> Result<(), String> {
    let workspace: Workspace = serde_json::from_str(contents)
        .map_err(|error| format!("The workspace data is invalid: {error}"))?;
    let folder_parents = workspace
        .folders
        .iter()
        .map(|folder| (folder.id.clone(), folder.parent_id.clone()))
        .collect::<Vec<_>>();
    let invalid_folders = mcp::invalid_folder_parent_ids(&folder_parents);
    if !invalid_folders.is_empty() {
        return Err(format!(
            "The workspace contains missing, self-referencing, or circular folder parents: {}",
            invalid_folders.join(", ")
        ));
    }
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start the save transaction: {error}"))?;

    transaction
        .execute_batch(
            "DELETE FROM folders;
             DELETE FROM todos;
             DELETE FROM todo_categories;
             DELETE FROM journal_entries;
             CREATE TEMP TABLE IF NOT EXISTS incoming_note_ids (id TEXT PRIMARY KEY);
             DELETE FROM incoming_note_ids;",
        )
        .map_err(|error| format!("Could not prepare the workspace save: {error}"))?;

    for (position, folder) in workspace.folders.iter().enumerate() {
        let icon = if folder.id == "inbox" {
            "ph-tray"
        } else {
            mcp::normalize_folder_icon(folder.icon.as_deref())
        };
        transaction
            .execute(
                "INSERT INTO folders (id, name, parent_id, is_open, icon, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    folder.id,
                    folder.name,
                    folder.parent_id,
                    folder.open,
                    icon,
                    position as i64
                ],
            )
            .map_err(|error| format!("Could not save a folder: {error}"))?;
    }

    for (position, note) in workspace.notes.iter().enumerate() {
        transaction
            .execute(
                "INSERT OR IGNORE INTO incoming_note_ids (id) VALUES (?1)",
                [&note.id],
            )
            .map_err(|error| format!("Could not stage a note save: {error}"))?;
        transaction
            .execute(
                "INSERT INTO notes (id, folder_id, title, content, updated, status, pinned, position, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   folder_id = excluded.folder_id,
                   title = excluded.title,
                   content = excluded.content,
                   updated = excluded.updated,
                   status = excluded.status,
                   pinned = excluded.pinned,
                   position = excluded.position,
                   revision = notes.revision + 1
                 WHERE excluded.updated >= notes.updated",
                params![
                    note.id,
                    note.folder_id,
                    note.title,
                    note.content,
                    note.updated,
                    note.status,
                    note.pinned,
                    position as i64,
                    note.revision
                ],
            )
            .map_err(|error| format!("Could not save a note: {error}"))?;
    }

    transaction
        .execute_batch(
            "DELETE FROM notes WHERE id NOT IN (SELECT id FROM incoming_note_ids);
             DROP TABLE incoming_note_ids;",
        )
        .map_err(|error| format!("Could not finalize note removals: {error}"))?;

    for (position, category) in workspace.todo_categories.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO todo_categories (id,name,color,icon,position) VALUES (?1,?2,?3,?4,?5)",
                params![
                    category.id,
                    category.name,
                    category.color,
                    category.icon,
                    position as i64
                ],
            )
            .map_err(|error| format!("Could not save a task category: {error}"))?;
    }
    if workspace.todo_categories.is_empty() {
        transaction.execute("INSERT OR IGNORE INTO todo_categories (id,name,color,icon,position) VALUES ('inbox','Inbox','#7b8e7c','ph-tray',0)", []).map_err(|error| format!("Could not seed category: {error}"))?;
    }
    for (position, todo) in workspace.todos.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO todos (id, text, completed, created, updated, position, category_id, priority, effort, color, scheduled_start, duration_minutes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    todo.id,
                    todo.text,
                    todo.completed,
                    todo.created,
                    todo.updated,
                    position as i64, todo.category_id, todo.priority, todo.effort, todo.color, todo.scheduled_start, todo.duration_minutes
                ],
            )
            .map_err(|error| format!("Could not save a task: {error}"))?;
    }
    for (position, entry) in workspace.journal_entries.iter().enumerate() {
        transaction.execute(
            "INSERT INTO journal_entries (id, date_key, content, created, updated, position) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![entry.id, entry.date_key, entry.content, entry.created, entry.updated, position as i64],
        ).map_err(|error| format!("Could not save a journal entry: {error}"))?;
    }

    transaction
        .execute(
            "INSERT INTO app_state (key, value) VALUES ('selectedFolderId', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [&workspace.selected_folder_id],
        )
        .map_err(|error| format!("Could not save the selected folder: {error}"))?;
    transaction
        .execute(
            "INSERT INTO app_state (key, value) VALUES ('selectedNoteId', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [&workspace.selected_note_id],
        )
        .map_err(|error| format!("Could not save the selected note: {error}"))?;
    transaction
        .execute(
            "INSERT INTO app_state (key, value) VALUES ('sortMode', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [&workspace.sort_mode],
        )
        .map_err(|error| format!("Could not save the note sort mode: {error}"))?;
    transaction.execute("INSERT INTO app_state (key,value) VALUES ('plannerView',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [&workspace.planner_view]).map_err(|error| format!("Could not save planner view: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("Could not commit the workspace save: {error}"))
}

fn read_workspace(connection: &Connection) -> Result<Option<Workspace>, String> {
    let folder_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM folders", [], |row| row.get(0))
        .map_err(|error| format!("Could not inspect the workspace database: {error}"))?;
    if folder_count == 0 {
        return Ok(None);
    }

    let mut folder_statement = connection
        .prepare("SELECT id, name, parent_id, is_open, icon FROM folders ORDER BY position")
        .map_err(|error| format!("Could not read folders: {error}"))?;
    let folders = folder_statement
        .query_map([], |row| {
            let id = row.get::<_, String>(0)?;
            let stored_icon = row.get::<_, Option<String>>(4)?;
            let icon = if id == "inbox" {
                Some(stored_icon.unwrap_or_else(|| "ph-tray".into()))
            } else {
                Some(mcp::normalize_folder_icon(stored_icon.as_deref()).into())
            };
            Ok(Folder {
                id,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                open: row.get(3)?,
                icon,
            })
        })
        .map_err(|error| format!("Could not query folders: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode folders: {error}"))?;

    let mut note_statement = connection
        .prepare(
            "SELECT id, folder_id, title, content, updated, status, pinned, revision
             FROM notes ORDER BY position",
        )
        .map_err(|error| format!("Could not read notes: {error}"))?;
    let notes = note_statement
        .query_map([], |row| {
            Ok(Note {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                title: row.get(2)?,
                content: row.get(3)?,
                updated: row.get(4)?,
                status: row.get(5)?,
                pinned: row.get(6)?,
                revision: row.get(7)?,
            })
        })
        .map_err(|error| format!("Could not query notes: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode notes: {error}"))?;

    let mut todo_statement = connection
        .prepare("SELECT id, text, completed, created, updated, category_id, priority, effort, color, scheduled_start, duration_minutes FROM todos ORDER BY position")
        .map_err(|error| format!("Could not read tasks: {error}"))?;
    let todos = todo_statement
        .query_map([], |row| {
            Ok(Todo {
                id: row.get(0)?,
                text: row.get(1)?,
                completed: row.get(2)?,
                created: row.get(3)?,
                updated: row.get(4)?,
                category_id: row.get(5)?,
                priority: row.get(6)?,
                effort: row.get(7)?,
                color: row.get(8)?,
                scheduled_start: row.get(9)?,
                duration_minutes: row.get(10)?,
            })
        })
        .map_err(|error| format!("Could not query tasks: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode tasks: {error}"))?;
    let mut category_statement = connection
        .prepare("SELECT id,name,color,icon FROM todo_categories ORDER BY position")
        .map_err(|error| format!("Could not read task categories: {error}"))?;
    let todo_categories = category_statement
        .query_map([], |row| {
            Ok(TodoCategory {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
            })
        })
        .map_err(|error| format!("Could not query task categories: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode categories: {error}"))?;
    let mut journal_statement = connection.prepare("SELECT id,date_key,content,created,updated FROM journal_entries ORDER BY date_key DESC, position").map_err(|error| format!("Could not read journal entries: {error}"))?;
    let journal_entries = journal_statement
        .query_map([], |row| {
            Ok(JournalEntry {
                id: row.get(0)?,
                date_key: row.get(1)?,
                content: row.get(2)?,
                created: row.get(3)?,
                updated: row.get(4)?,
            })
        })
        .map_err(|error| format!("Could not query journal entries: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode journal entries: {error}"))?;

    let selected_folder_id = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = 'selectedFolderId'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read the selected folder: {error}"))?
        .unwrap_or_else(|| folders[0].id.clone());
    let selected_note_id = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = 'selectedNoteId'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read the selected note: {error}"))?
        .unwrap_or_else(|| {
            notes
                .first()
                .map(|note| note.id.clone())
                .unwrap_or_default()
        });
    let sort_mode = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = 'sortMode'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read the note sort mode: {error}"))?
        .unwrap_or_else(default_sort_mode);
    let planner_view = connection
        .query_row(
            "SELECT value FROM app_state WHERE key='plannerView'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not read planner view: {error}"))?
        .unwrap_or_else(default_planner_view);

    Ok(Some(Workspace {
        folders,
        notes,
        todos,
        todo_categories,
        journal_entries,
        selected_folder_id,
        selected_note_id,
        sort_mode,
        planner_view,
    }))
}

fn backup_database(connection: &Connection, destination: &Path) -> Result<(), String> {
    connection
        .backup("main", destination, None)
        .map_err(|error| format!("Could not back up the workspace: {error}"))
}

fn maybe_create_backup(app: &AppHandle, connection: &Connection) -> Result<(), String> {
    let destination = backup_directory(app)?.join("workspace-latest.sqlite3");
    let fresh = fs::metadata(&destination)
        .and_then(|metadata| metadata.modified())
        .and_then(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .map_err(std::io::Error::other)
        })
        .map(|age| age < Duration::from_secs(60 * 60))
        .unwrap_or(false);
    if !fresh {
        backup_database(connection, &destination)?;
    }
    Ok(())
}

#[tauri::command]
fn load_workspace(app: AppHandle) -> Result<Option<String>, String> {
    let connection = open_database(&app)?;
    if let Some(workspace) = read_workspace(&connection)? {
        maybe_create_backup(&app, &connection)?;
        return serde_json::to_string(&workspace)
            .map(Some)
            .map_err(|error| format!("Could not encode the workspace: {error}"));
    }

    Ok(None)
}

#[tauri::command]
fn load_note(app: AppHandle, note_id: String) -> Result<Option<Note>, String> {
    let connection = open_database(&app)?;
    connection
        .query_row(
            "SELECT id, folder_id, title, content, updated, status, pinned, revision
             FROM notes WHERE id = ?1",
            [&note_id],
            |row| {
                Ok(Note {
                    id: row.get(0)?,
                    folder_id: row.get(1)?,
                    title: row.get(2)?,
                    content: row.get(3)?,
                    updated: row.get(4)?,
                    status: row.get(5)?,
                    pinned: row.get(6)?,
                    revision: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("Could not load the note: {error}"))
}

fn note_window_label(note_id: &str) -> String {
    let suffix: String = note_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect();
    format!("note-{suffix}")
}

#[tauri::command]
fn list_detached_notes(app: AppHandle) -> Vec<String> {
    app.webview_windows()
        .into_keys()
        .filter_map(|label| label.strip_prefix("note-").map(str::to_owned))
        .collect()
}

#[tauri::command]
fn attach_note_to_main(app: AppHandle, note_id: String) -> Result<Option<Note>, String> {
    let label = note_window_label(&note_id);
    if let Some(window) = app.get_webview_window(&label) {
        window
            .close()
            .map_err(|error| format!("Could not close the detached note window: {error}"))?;
    }
    let note = load_note(app.clone(), note_id.clone())?;
    app.emit("note-attached", &note_id)
        .map_err(|error| format!("Could not notify the main window: {error}"))?;
    Ok(note)
}

#[tauri::command]
fn save_note(app: AppHandle, note: Note) -> Result<i64, String> {
    let connection = open_database(&app)?;
    let updated = connection
        .execute(
            "UPDATE notes
             SET folder_id = ?2, title = ?3, content = ?4, updated = ?5,
                 status = ?6, pinned = ?7, revision = revision + 1
             WHERE id = ?1 AND revision = ?8",
            params![
                note.id,
                note.folder_id,
                note.title,
                note.content,
                note.updated,
                note.status,
                note.pinned,
                note.revision
            ],
        )
        .map_err(|error| format!("Could not save the note: {error}"))?;
    if updated == 0 {
        return Err("The note no longer exists or a newer edit is already saved.".into());
    }
    let revision = connection
        .query_row(
            "SELECT revision FROM notes WHERE id = ?1",
            [&note.id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not read the saved note revision: {error}"))?;
    maybe_create_backup(&app, &connection)?;
    let saved = Note { revision, ..note };
    app.emit("note-updated", &saved)
        .map_err(|error| format!("Could not notify other windows: {error}"))?;
    Ok(revision)
}

#[tauri::command]
async fn open_note_window(app: AppHandle, note_id: String, title: String) -> Result<(), String> {
    let label = note_window_label(&note_id);
    if let Some(window) = app.get_webview_window(&label) {
        window
            .set_focus()
            .map_err(|error| format!("Could not focus the note window: {error}"))?;
        return Ok(());
    }

    let detached_note_id = note_id.clone();
    let event_app = app.clone();
    let window = WebviewWindowBuilder::new(
        &app,
        label,
        WebviewUrl::App(format!("index.html?note={note_id}").into()),
    )
    .title(if title.trim().is_empty() {
        "Untitled — Odo".to_string()
    } else {
        format!("{title} — Odo")
    })
    .inner_size(760.0, 760.0)
    .min_inner_size(520.0, 420.0)
    .center()
    .build()
    .map_err(|error| format!("Could not open the note window: {error}"))?;
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = event_app.emit("note-attached", &detached_note_id);
        }
    });
    app.emit("note-detached", &note_id)
        .map_err(|error| format!("Could not notify the main window: {error}"))?;
    Ok(())
}

#[tauri::command]
fn save_workspace(app: AppHandle, contents: String) -> Result<(), String> {
    let mut connection = open_database(&app)?;
    write_workspace(&mut connection, &contents)?;
    maybe_create_backup(&app, &connection)
}

#[tauri::command]
fn create_backup(app: AppHandle) -> Result<String, String> {
    let connection = open_database(&app)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("Could not create a backup timestamp: {error}"))?
        .as_secs();
    let destination = backup_directory(&app)?.join(format!("workspace-{timestamp}.sqlite3"));
    backup_database(&connection, &destination)?;
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
fn get_storage_info(app: AppHandle) -> Result<StorageInfo, String> {
    Ok(StorageInfo {
        database_path: database_path(&app)?.to_string_lossy().into_owned(),
        backup_directory: backup_directory(&app)?.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
fn export_note(app: AppHandle, title: String, contents: String) -> Result<String, String> {
    let documents = app
        .path()
        .document_dir()
        .map_err(|error| format!("Could not locate the Documents directory: {error}"))?;
    let directory = documents.join("Odo Exports");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the export directory: {error}"))?;
    let safe_title: String = title
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            other => other,
        })
        .collect();
    let path = directory.join(format!("{}.md", safe_title.trim().trim_end_matches('.')));
    fs::write(&path, contents).map_err(|error| format!("Could not export the note: {error}"))?;
    Ok(path.to_string_lossy().into_owned())
}

struct McpRuntime {
    database_path: PathBuf,
    status: Arc<Mutex<mcp::McpStatus>>,
    cancellation: Mutex<Option<tokio_util::sync::CancellationToken>>,
}

impl McpRuntime {
    fn new(database_path: PathBuf) -> Self {
        Self {
            database_path,
            status: Arc::new(Mutex::new(mcp::McpStatus::default())),
            cancellation: Mutex::new(None),
        }
    }

    fn restart(&self, app: &AppHandle, config: mcp::McpConfig) {
        if let Some(cancellation) = self
            .cancellation
            .lock()
            .expect("lock MCP cancellation")
            .take()
        {
            cancellation.cancel();
        }
        *self.status.lock().expect("lock MCP status") = mcp::McpStatus::default();
        refresh_tray(app, &config, &self.status.lock().expect("lock MCP status"));
        let _ = app.emit("mcp-status-changed", ());
        if !config.enabled {
            return;
        }

        let cancellation = tokio_util::sync::CancellationToken::new();
        *self.cancellation.lock().expect("lock MCP cancellation") = Some(cancellation.clone());
        let database_path = self.database_path.clone();
        let status = self.status.clone();
        let event_app = app.clone();
        let server_app = app.clone();
        std::thread::Builder::new()
            .name("odo-mcp-http".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .thread_name("odo-mcp-worker")
                    .build()
                    .expect("build Odo MCP runtime");
                runtime.block_on(async move {
                    let watcher_app = server_app.clone();
                    let watcher_config = config.clone();
                    let watcher_status = status.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(120)).await;
                        refresh_tray(
                            &watcher_app,
                            &watcher_config,
                            &watcher_status.lock().expect("lock MCP status"),
                        );
                        let _ = watcher_app.emit("mcp-status-changed", ());
                    });
                    let notifier: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
                        let _ = event_app.emit("workspace-changed", ());
                    });
                    if let Err(error) = mcp::run_http(
                        database_path,
                        config.clone(),
                        cancellation,
                        Some(notifier),
                        status.clone(),
                    )
                    .await
                    {
                        *status.lock().expect("lock MCP status") = mcp::McpStatus {
                            running: false,
                            endpoint: None,
                            error: Some(error),
                        };
                    }
                    refresh_tray(
                        &server_app,
                        &config,
                        &status.lock().expect("lock MCP status"),
                    );
                    let _ = server_app.emit("mcp-status-changed", ());
                });
            })
            .expect("spawn Odo MCP thread");
    }
}

struct TrayHandles {
    toggle: CheckMenuItem<tauri::Wry>,
    status: MenuItem<tauri::Wry>,
}

fn refresh_tray(app: &AppHandle, config: &mcp::McpConfig, status: &mcp::McpStatus) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let _ = handles.toggle.set_checked(config.enabled);
    let text = if let Some(error) = &status.error {
        format!("MCP error: {}", error.chars().take(54).collect::<String>())
    } else if let Some(endpoint) = &status.endpoint {
        format!("MCP running — {endpoint}")
    } else if config.enabled {
        "MCP starting…".into()
    } else {
        "MCP stopped".into()
    };
    let _ = handles.status.set_text(text);
    if let Some(tray) = app.tray_by_id("odo-menu") {
        let _ = tray.set_tooltip(Some(if status.running {
            "Odo — MCP running"
        } else {
            "Odo — MCP stopped"
        }));
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn toggle_mcp_from_tray(app: &AppHandle) {
    let runtime = app.state::<McpRuntime>();
    if let Ok(mut config) = mcp::load_config(&runtime.database_path) {
        config.enabled = !config.enabled;
        if mcp::save_config(&runtime.database_path, &config).is_ok() {
            runtime.restart(app, config);
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct McpSettingsResponse {
    config: mcp::McpConfig,
    status: mcp::McpStatus,
    masked_token: String,
    http_snippet: String,
    stdio_snippet: String,
}

fn masked_token(token: &str) -> String {
    if token.len() <= 8 {
        return "•".repeat(token.len().max(8));
    }
    format!(
        "{}{}{}",
        &token[..4],
        "•".repeat(12),
        &token[token.len() - 4..]
    )
}

#[tauri::command]
fn get_mcp_settings(runtime: tauri::State<'_, McpRuntime>) -> Result<McpSettingsResponse, String> {
    let config = mcp::load_config(&runtime.database_path)?;
    let status = runtime.status.lock().expect("lock MCP status").clone();
    let endpoint = status
        .endpoint
        .clone()
        .unwrap_or_else(|| format!("http://{}:{}/mcp", config.host, config.port));
    let http_value = if config.auth_enabled {
        json!({"mcpServers":{"odo":{"url":endpoint,"headers":{"Authorization":format!("Bearer {}",config.token)}}}})
    } else {
        json!({"mcpServers":{"odo":{"url":endpoint}}})
    };
    let executable = std::env::current_exe()
        .map_err(|error| format!("Could not locate the Odo executable: {error}"))?;
    let stdio_value = json!({"mcpServers":{"odo":{"command":executable,"args":["--mcp-stdio"],"env":{"ODO_DATABASE_PATH":runtime.database_path}}}});
    Ok(McpSettingsResponse {
        masked_token: masked_token(&config.token),
        http_snippet: serde_json::to_string_pretty(&http_value)
            .map_err(|error| error.to_string())?,
        stdio_snippet: serde_json::to_string_pretty(&stdio_value)
            .map_err(|error| error.to_string())?,
        config,
        status,
    })
}

#[tauri::command]
fn update_mcp_settings(
    app: AppHandle,
    runtime: tauri::State<'_, McpRuntime>,
    config: mcp::McpConfig,
) -> Result<(), String> {
    if config.auth_enabled && config.token.trim().is_empty() {
        return Err("Authentication is enabled, so a token is required".into());
    }
    if config.start_at_login {
        app.autolaunch()
            .enable()
            .map_err(|error| format!("Could not enable start at login: {error}"))?;
    } else {
        app.autolaunch()
            .disable()
            .map_err(|error| format!("Could not disable start at login: {error}"))?;
    }
    mcp::save_config(&runtime.database_path, &config)?;
    runtime.restart(&app, config);
    Ok(())
}

#[tauri::command]
fn generate_mcp_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[tauri::command]
fn get_mcp_change_version(runtime: tauri::State<'_, McpRuntime>) -> Result<i64, String> {
    mcp::change_version(&runtime.database_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            let database_path = database_path(&handle).map_err(std::io::Error::other)?;
            let connection = open_database(&handle).map_err(std::io::Error::other)?;
            mcp::ensure_support_schema(&connection).map_err(std::io::Error::other)?;
            app.manage(McpRuntime::new(database_path.clone()));

            let open_item = MenuItem::with_id(app, "open", "Open Odo", true, None::<&str>)?;
            let settings_item =
                MenuItem::with_id(app, "settings", "Open Settings", true, None::<&str>)?;
            let separator_one = PredefinedMenuItem::separator(app)?;
            let config = mcp::load_config(&database_path).map_err(std::io::Error::other)?;
            let toggle_item = CheckMenuItem::with_id(
                app,
                "toggle-mcp",
                "MCP Server",
                true,
                config.enabled,
                None::<&str>,
            )?;
            let status_item =
                MenuItem::with_id(app, "mcp-status", "MCP stopped", false, None::<&str>)?;
            let separator_two = PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Odo", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &open_item,
                    &settings_item,
                    &separator_one,
                    &toggle_item,
                    &status_item,
                    &separator_two,
                    &quit_item,
                ],
            )?;
            let tray = TrayIconBuilder::with_id("odo-menu")
                .icon(app.default_window_icon().expect("Odo app icon").clone())
                .icon_as_template(true)
                .tooltip("Odo")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show_main(app),
                    "settings" => {
                        show_main(app);
                        let _ = app.emit("open-settings", ());
                    }
                    "toggle-mcp" => toggle_mcp_from_tray(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            let _ = tray;
            app.manage(TrayHandles {
                toggle: toggle_item,
                status: status_item,
            });
            app.state::<McpRuntime>().restart(&handle, config);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_workspace,
            save_workspace,
            load_note,
            save_note,
            open_note_window,
            list_detached_notes,
            attach_note_to_main,
            create_backup,
            get_storage_info,
            export_note,
            get_mcp_settings,
            update_mcp_settings,
            generate_mcp_token,
            get_mcp_change_version
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    app.run(|_app, _event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen {
            has_visible_windows,
            ..
        } = _event
        {
            if !has_visible_windows {
                show_main(_app);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_round_trip_preserves_notes_folders_and_tasks() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        initialize_database(&connection).expect("initialize schema");
        let input = r###"{
          "folders":[{"id":"inbox","name":"Inbox","parentId":null,"open":true,"icon":"ph-tray"}],
          "notes":[{"id":"note-1","folderId":"inbox","title":"Test","content":"Body","updated":"2026-07-15T00:00:00.000Z","status":"active","pinned":true}],
          "todos":[{"id":"todo-1","text":"Ship it","completed":false,"created":"2026-07-15T00:00:00.000Z","updated":"2026-07-15T00:00:00.000Z","categoryId":"work","priority":"high","effort":4,"color":"#7499b1","scheduledStart":"2026-07-15T09:00:00.000Z","durationMinutes":90}],
          "todoCategories":[{"id":"inbox","name":"Inbox","color":"#7b8e7c"},{"id":"work","name":"Work","color":"#7499b1"}],
          "journalEntries":[{"id":"journal-1","dateKey":"2026-07-15","content":"## 9:30 AM\n\nA calm start.","created":"2026-07-15T09:30:00.000Z","updated":"2026-07-15T09:30:00.000Z"}],
          "selectedFolderId":"inbox",
          "selectedNoteId":"note-1",
          "sortMode":"manual", "plannerView":"4"
        }"###;

        write_workspace(&mut connection, input).expect("save workspace");
        let workspace = read_workspace(&connection)
            .expect("read workspace")
            .expect("workspace exists");

        assert_eq!(workspace.folders.len(), 1);
        assert_eq!(workspace.notes.len(), 1);
        assert_eq!(workspace.todos.len(), 1);
        assert_eq!(workspace.notes[0].title, "Test");
        assert_eq!(workspace.todos[0].text, "Ship it");
        assert_eq!(
            workspace.todos[0].scheduled_start.as_deref(),
            Some("2026-07-15T09:00:00.000Z")
        );
        assert_eq!(workspace.todos[0].duration_minutes, 90);
        assert_eq!(workspace.todo_categories.len(), 2);
        assert_eq!(workspace.journal_entries.len(), 1);
        assert_eq!(workspace.journal_entries[0].date_key, "2026-07-15");
        assert_eq!(workspace.planner_view, "4");
        assert_eq!(workspace.selected_note_id, "note-1");
        assert_eq!(workspace.sort_mode, "manual");
    }

    #[test]
    fn stale_workspace_snapshot_cannot_overwrite_a_newer_note_revision() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        initialize_database(&connection).expect("initialize schema");
        let stale_snapshot = r#"{
          "folders":[{"id":"inbox","name":"Inbox","parentId":null,"open":true,"icon":"ph-tray"}],
          "notes":[{"id":"note-1","folderId":"inbox","title":"Old","content":"Old body","updated":"2026-07-15T00:00:00.000Z","status":"active","pinned":false,"revision":0}],
          "todos":[],
          "selectedFolderId":"inbox",
          "selectedNoteId":"note-1"
        }"#;

        write_workspace(&mut connection, stale_snapshot).expect("save initial workspace");
        connection
            .execute(
                "UPDATE notes SET title = 'New', content = 'New body',
                 updated = '2026-07-15T00:01:00.000Z', revision = 3
                 WHERE id = 'note-1'",
                [],
            )
            .expect("simulate detached note save");

        write_workspace(&mut connection, stale_snapshot).expect("save stale workspace snapshot");
        let workspace = read_workspace(&connection)
            .expect("read workspace")
            .expect("workspace exists");

        assert_eq!(workspace.notes[0].title, "New");
        assert_eq!(workspace.notes[0].content, "New body");
        assert_eq!(workspace.notes[0].revision, 3);
    }

    #[test]
    fn workspace_save_rejects_circular_folder_parents_before_starting_a_transaction() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        initialize_database(&connection).expect("initialize schema");
        let circular = r#"{
          "folders":[
            {"id":"inbox","name":"Inbox","parentId":null,"open":true,"icon":"ph-tray"},
            {"id":"a","name":"A","parentId":"c","open":true,"icon":"ph-folder"},
            {"id":"b","name":"B","parentId":"a","open":true,"icon":"ph-folder"},
            {"id":"c","name":"C","parentId":"b","open":true,"icon":"ph-folder"}
          ],
          "notes":[],
          "todos":[],
          "selectedFolderId":"inbox",
          "selectedNoteId":""
        }"#;
        let error = write_workspace(&mut connection, circular).unwrap_err();
        assert!(error.contains("circular folder parents"));
        connection
            .execute_batch("BEGIN IMMEDIATE; ROLLBACK;")
            .expect("rejected save must not leave a write lock");
    }
}
