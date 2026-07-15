use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

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
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Workspace {
    folders: Vec<Folder>,
    notes: Vec<Note>,
    #[serde(default)]
    todos: Vec<Todo>,
    selected_folder_id: String,
    selected_note_id: String,
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
    initialize_database(&connection)?;
    Ok(connection)
}

fn initialize_database(connection: &Connection) -> Result<(), String> {
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
             );",
        )
        .map_err(|error| format!("Could not initialize the workspace database: {error}"))?;
    Ok(())
}

fn write_workspace(connection: &mut Connection, contents: &str) -> Result<(), String> {
    let workspace: Workspace = serde_json::from_str(contents)
        .map_err(|error| format!("The workspace data is invalid: {error}"))?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start the save transaction: {error}"))?;

    transaction
        .execute_batch("DELETE FROM folders; DELETE FROM notes; DELETE FROM todos;")
        .map_err(|error| format!("Could not prepare the workspace save: {error}"))?;

    for (position, folder) in workspace.folders.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO folders (id, name, parent_id, is_open, icon, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    folder.id,
                    folder.name,
                    folder.parent_id,
                    folder.open,
                    folder.icon,
                    position as i64
                ],
            )
            .map_err(|error| format!("Could not save a folder: {error}"))?;
    }

    for (position, note) in workspace.notes.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO notes (id, folder_id, title, content, updated, status, pinned, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    note.id,
                    note.folder_id,
                    note.title,
                    note.content,
                    note.updated,
                    note.status,
                    note.pinned,
                    position as i64
                ],
            )
            .map_err(|error| format!("Could not save a note: {error}"))?;
    }

    for (position, todo) in workspace.todos.iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO todos (id, text, completed, created, updated, position)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    todo.id,
                    todo.text,
                    todo.completed,
                    todo.created,
                    todo.updated,
                    position as i64
                ],
            )
            .map_err(|error| format!("Could not save a task: {error}"))?;
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
            Ok(Folder {
                id: row.get(0)?,
                name: row.get(1)?,
                parent_id: row.get(2)?,
                open: row.get(3)?,
                icon: row.get(4)?,
            })
        })
        .map_err(|error| format!("Could not query folders: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode folders: {error}"))?;

    let mut note_statement = connection
        .prepare(
            "SELECT id, folder_id, title, content, updated, status, pinned
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
            })
        })
        .map_err(|error| format!("Could not query notes: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode notes: {error}"))?;

    let mut todo_statement = connection
        .prepare("SELECT id, text, completed, created, updated FROM todos ORDER BY position")
        .map_err(|error| format!("Could not read tasks: {error}"))?;
    let todos = todo_statement
        .query_map([], |row| {
            Ok(Todo {
                id: row.get(0)?,
                text: row.get(1)?,
                completed: row.get(2)?,
                created: row.get(3)?,
                updated: row.get(4)?,
            })
        })
        .map_err(|error| format!("Could not query tasks: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode tasks: {error}"))?;

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
        .unwrap_or_else(|| notes.first().map(|note| note.id.clone()).unwrap_or_default());

    Ok(Some(Workspace {
        folders,
        notes,
        todos,
        selected_folder_id,
        selected_note_id,
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
        .and_then(|modified| SystemTime::now().duration_since(modified).map_err(std::io::Error::other))
        .map(|age| age < Duration::from_secs(60 * 60))
        .unwrap_or(false);
    if !fresh {
        backup_database(connection, &destination)?;
    }
    Ok(())
}

#[tauri::command]
fn load_workspace(app: AppHandle) -> Result<Option<String>, String> {
    let mut connection = open_database(&app)?;
    if let Some(workspace) = read_workspace(&connection)? {
        maybe_create_backup(&app, &connection)?;
        return serde_json::to_string(&workspace)
            .map(Some)
            .map_err(|error| format!("Could not encode the workspace: {error}"));
    }

    let legacy_path = app_data_dir(&app)?.join("workspace.json");
    if legacy_path.exists() {
        let contents = fs::read_to_string(&legacy_path)
            .map_err(|error| format!("Could not read the previous workspace: {error}"))?;
        write_workspace(&mut connection, &contents)?;
        let _ = fs::rename(&legacy_path, legacy_path.with_extension("json.migrated"));
        return Ok(Some(contents));
    }

    Ok(None)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            load_workspace,
            save_workspace,
            create_backup,
            get_storage_info,
            export_note
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_round_trip_preserves_notes_folders_and_tasks() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        initialize_database(&connection).expect("initialize schema");
        let input = r#"{
          "folders":[{"id":"inbox","name":"Inbox","parentId":null,"open":true,"icon":"ph-tray"}],
          "notes":[{"id":"note-1","folderId":"inbox","title":"Test","content":"Body","updated":"2026-07-15T00:00:00.000Z","status":"active","pinned":true}],
          "todos":[{"id":"todo-1","text":"Ship it","completed":false,"created":"2026-07-15T00:00:00.000Z","updated":"2026-07-15T00:00:00.000Z"}],
          "selectedFolderId":"inbox",
          "selectedNoteId":"note-1"
        }"#;

        write_workspace(&mut connection, input).expect("save workspace");
        let workspace = read_workspace(&connection)
            .expect("read workspace")
            .expect("workspace exists");

        assert_eq!(workspace.folders.len(), 1);
        assert_eq!(workspace.notes.len(), 1);
        assert_eq!(workspace.todos.len(), 1);
        assert_eq!(workspace.notes[0].title, "Test");
        assert_eq!(workspace.todos[0].text, "Ship it");
        assert_eq!(workspace.selected_note_id, "note-1");
    }
}
