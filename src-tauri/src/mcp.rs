use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use chrono::{DateTime, NaiveDate};
use rmcp::{
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    prompt, prompt_handler, prompt_router,
    schemars::{JsonSchema, Schema, SchemaGenerator},
    service::{Peer, RequestContext},
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

const CONFIG_KEY: &str = "mcp_config_v1";
const FOLDER_HIERARCHY_REPAIR_KEY: &str = "mcp_folder_hierarchy_repaired_v2";
const RETENTION_SECONDS: i64 = 60 * 24 * 60 * 60;
pub const DEFAULT_FOLDER_ICON: &str = "ph-folder";
pub const ALLOWED_FOLDER_ICONS: [&str; 50] = [
    "ph-folder",
    "ph-briefcase",
    "ph-user-circle",
    "ph-book",
    "ph-book-open",
    "ph-notebook",
    "ph-file-text",
    "ph-lightbulb",
    "ph-star",
    "ph-heart",
    "ph-house",
    "ph-buildings",
    "ph-bank",
    "ph-graduation-cap",
    "ph-flask",
    "ph-code",
    "ph-terminal-window",
    "ph-globe",
    "ph-map-pin",
    "ph-airplane",
    "ph-car",
    "ph-bicycle",
    "ph-camera",
    "ph-image",
    "ph-music-note",
    "ph-film-strip",
    "ph-game-controller",
    "ph-palette",
    "ph-paint-brush",
    "ph-tree",
    "ph-leaf",
    "ph-flower",
    "ph-sun",
    "ph-moon",
    "ph-cloud",
    "ph-mountains",
    "ph-coffee",
    "ph-cooking-pot",
    "ph-barbell",
    "ph-basketball",
    "ph-soccer-ball",
    "ph-paw-print",
    "ph-gift",
    "ph-shopping-bag",
    "ph-currency-dollar",
    "ph-chart-line-up",
    "ph-calendar-blank",
    "ph-clock",
    "ph-tag",
    "ph-planet",
];

pub fn normalize_folder_icon(icon: Option<&str>) -> &'static str {
    icon.and_then(|value| {
        ALLOWED_FOLDER_ICONS
            .iter()
            .copied()
            .find(|allowed| *allowed == value)
    })
    .unwrap_or(DEFAULT_FOLDER_ICON)
}

fn validate_folder_icon(icon: Option<String>) -> Result<String, String> {
    match icon {
        Some(icon) if ALLOWED_FOLDER_ICONS.contains(&icon.as_str()) => Ok(icon),
        Some(icon) => Err(format!(
            "Folder icon '{icon}' is not allowed. Use one of the 50 values returned by folders_list.allowedIcons; action icons such as archive, trash, delete, add, and edit are reserved."
        )),
        None => Ok(DEFAULT_FOLDER_ICON.into()),
    }
}

fn folder_icon_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = Option::<String>::json_schema(generator);
    schema.insert(
        "enum".into(),
        Value::Array(
            ALLOWED_FOLDER_ICONS
                .iter()
                .map(|icon| Value::String((*icon).into()))
                .collect(),
        ),
    );
    schema
}

pub fn invalid_folder_parent_ids(folders: &[(String, Option<String>)]) -> Vec<String> {
    let ids = folders
        .iter()
        .map(|(id, _)| id.as_str())
        .collect::<HashSet<_>>();
    let mut parents = folders.iter().cloned().collect::<HashMap<_, _>>();
    let mut invalid = Vec::new();

    for (id, parent) in folders {
        if (id == "inbox" && parent.is_some())
            || parent.as_deref() == Some(id.as_str())
            || parent
                .as_deref()
                .is_some_and(|parent| !ids.contains(parent))
        {
            parents.insert(id.clone(), None);
            invalid.push(id.clone());
        }
    }

    for (id, _) in folders {
        let mut path = HashSet::new();
        let mut cursor = id.clone();
        loop {
            if !path.insert(cursor.clone()) {
                if !invalid.contains(&cursor) {
                    invalid.push(cursor.clone());
                }
                parents.insert(cursor, None);
                break;
            }
            match parents.get(&cursor).cloned().flatten() {
                Some(parent) => cursor = parent,
                None => break,
            }
        }
    }
    invalid
}

fn repair_folder_hierarchy(connection: &Connection) -> Result<usize, String> {
    let mut statement = connection
        .prepare("SELECT id,parent_id FROM folders ORDER BY position,id")
        .map_err(|error| format!("Could not inspect the folder hierarchy: {error}"))?;
    let folders = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| format!("Could not read the folder hierarchy: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not decode the folder hierarchy: {error}"))?;
    drop(statement);
    let invalid_ids = invalid_folder_parent_ids(&folders);
    for id in &invalid_ids {
        connection
            .execute("UPDATE folders SET parent_id=NULL WHERE id=?1", [id])
            .map_err(|error| format!("Could not repair folder '{id}': {error}"))?;
    }
    Ok(invalid_ids.len())
}

fn validate_folder_parent(
    connection: &Connection,
    folder_id: Option<&str>,
    parent_id: Option<&str>,
) -> Result<(), String> {
    let Some(parent_id) = parent_id else {
        return Ok(());
    };
    if folder_id == Some("inbox") {
        return Err("Inbox must remain a root folder".into());
    }
    if folder_id == Some(parent_id) {
        return Err("A folder cannot be its own parent".into());
    }
    let parent_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE id=?1)",
            [parent_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not validate the parent folder: {error}"))?;
    if !parent_exists {
        return Err(format!("Parent folder '{parent_id}' was not found"));
    }
    if let Some(folder_id) = folder_id {
        let creates_cycle: bool = connection
            .query_row(
                "WITH RECURSIVE ancestors(id,parent_id) AS (
                   SELECT id,parent_id FROM folders WHERE id=?1
                   UNION
                   SELECT f.id,f.parent_id FROM folders f JOIN ancestors a ON f.id=a.parent_id
                 )
                 SELECT EXISTS(SELECT 1 FROM ancestors WHERE id=?2)",
                params![parent_id, folder_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("Could not validate the folder hierarchy: {error}"))?;
        if creates_cycle {
            return Err(format!(
                "Moving folder '{folder_id}' under '{parent_id}' would create a circular folder reference"
            ));
        }
    }
    Ok(())
}

fn normalize_folder_parent_for_create(
    connection: &Connection,
    parent_id: Option<String>,
) -> Result<Option<String>, String> {
    let Some(parent_id) = parent_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let parent_exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM folders WHERE id=?1)",
            [&parent_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not validate the parent folder: {error}"))?;
    Ok(parent_exists.then_some(parent_id))
}

const MAX_NOTE_TITLE_CHARS: usize = 1_000;
const MAX_FOLDER_NAME_CHARS: usize = 1_000;
const MAX_TASK_DURATION_MINUTES: i64 = 24 * 60;

fn validate_note_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty() {
        return Err("Note title cannot be empty".into());
    }
    if title.chars().count() > MAX_NOTE_TITLE_CHARS {
        return Err(format!(
            "Note title cannot exceed {MAX_NOTE_TITLE_CHARS} characters"
        ));
    }
    Ok(())
}

fn validate_folder_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Folder name cannot be empty".into());
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(format!(
            "Folder name cannot exceed {MAX_FOLDER_NAME_CHARS} characters"
        ));
    }
    Ok(name)
}

fn validate_journal_date_key(date_key: &str) -> Result<(), String> {
    let bytes = date_key.as_bytes();
    let exact_shape = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if !exact_shape || NaiveDate::parse_from_str(date_key, "%Y-%m-%d").is_err() {
        return Err("Journal dateKey must be a valid date in YYYY-MM-DD format".into());
    }
    Ok(())
}

fn validate_task_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err("Task text cannot be empty".into());
    }
    Ok(())
}

fn validate_task_priority(priority: Option<String>) -> Result<String, String> {
    let priority = priority.unwrap_or_else(|| "medium".into());
    if !matches!(priority.as_str(), "low" | "medium" | "high") {
        return Err("Task priority must be one of: low, medium, high".into());
    }
    Ok(priority)
}

fn validate_task_color(color: Option<String>) -> Result<String, String> {
    let Some(color) = color else {
        return Ok(String::new());
    };
    if color.len() != 7
        || !color.starts_with('#')
        || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Task color must be a six-digit hex color such as #7b8e7c".into());
    }
    Ok(color)
}

fn validate_scheduled_start(scheduled_start: Option<String>) -> Result<Option<String>, String> {
    let Some(scheduled_start) = scheduled_start else {
        return Ok(None);
    };
    if DateTime::parse_from_rfc3339(&scheduled_start).is_err() {
        return Err("Task scheduledStart must be a timezone-qualified ISO-8601 timestamp".into());
    }
    Ok(Some(scheduled_start))
}

fn normalize_task_duration(duration_minutes: Option<i64>) -> Result<i64, String> {
    let duration = duration_minutes.unwrap_or(30);
    if duration <= 0 {
        return Err("Task durationMinutes must be greater than zero".into());
    }
    Ok(duration.min(MAX_TASK_DURATION_MINUTES))
}

fn task_priority_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = Option::<String>::json_schema(generator);
    schema.insert("enum".into(), json!(["low", "medium", "high"]));
    schema
}

fn task_color_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = Option::<String>::json_schema(generator);
    schema.insert("pattern".into(), json!(r"^#[0-9A-Fa-f]{6}$"));
    schema
}

fn scheduled_start_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = Option::<String>::json_schema(generator);
    schema.insert("format".into(), json!("date-time"));
    schema
}

fn journal_date_schema(generator: &mut SchemaGenerator) -> Schema {
    let mut schema = String::json_schema(generator);
    schema.insert("pattern".into(), json!(r"^\d{4}-\d{2}-\d{2}$"));
    schema
}

fn backup_identifier(timestamp: i64) -> String {
    format!("workspace-mcp-{timestamp}.sqlite3")
}

fn empty_paginated(collection: &str, offset: u32) -> Value {
    let mut value = json!({
        "limit": 0,
        "offset": offset,
        "hasMore": false,
        "nextOffset": Value::Null,
    });
    value[collection] = json!([]);
    value
}

fn delete_folder_tree(connection: &mut Connection, id: &str) -> Result<usize, String> {
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Could not start folder deletion: {error}"))?;
    transaction
        .execute(
            "WITH RECURSIVE tree(id) AS (
               SELECT ?1
               UNION
               SELECT f.id FROM folders f JOIN tree t ON f.parent_id=t.id
             )
             UPDATE notes SET folder_id='inbox',status='trash',updated=strftime('%Y-%m-%dT%H:%M:%fZ','now'),revision=revision+1
             WHERE folder_id IN tree",
            [id],
        )
        .map_err(|error| format!("Could not move folder notes to Trash: {error}"))?;
    let deleted = transaction
        .execute(
            "WITH RECURSIVE tree(id) AS (
               SELECT ?1
               UNION
               SELECT f.id FROM folders f JOIN tree t ON f.parent_id=t.id
             )
             DELETE FROM folders WHERE id IN tree",
            [id],
        )
        .map_err(|error| format!("Could not delete the folder tree: {error}"))?;
    if deleted == 0 {
        return Err("Folder not found".into());
    }
    transaction
        .commit()
        .map_err(|error| format!("Could not finish folder deletion: {error}"))?;
    Ok(deleted)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub auth_enabled: bool,
    pub token: String,
    pub permanent_delete_enabled: bool,
    pub start_at_login: bool,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: 8765,
            auth_enabled: false,
            token: uuid::Uuid::new_v4().simple().to_string(),
            permanent_delete_enabled: false,
            start_at_login: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub running: bool,
    pub endpoint: Option<String>,
    pub error: Option<String>,
}

pub fn default_database_path() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("ODO_DATABASE_PATH") {
        return Ok(PathBuf::from(path));
    }
    dirs::data_dir()
        .map(|path| path.join("com.odotasks.desktop").join("workspace.sqlite3"))
        .ok_or_else(|| "Could not locate the operating system data directory".into())
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn now_iso() -> String {
    // SQLite creates an RFC3339-compatible UTC timestamp without requiring another clock crate.
    let connection = Connection::open_in_memory().expect("open timestamp database");
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .unwrap_or_else(|_| "1970-01-01T00:00:00.000Z".into())
}

fn open(path: &Path) -> Result<Connection, String> {
    let connection = Connection::open(path).map_err(|error| {
        format!(
            "Could not open the Odo database at {}: {error}",
            path.display()
        )
    })?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|error| format!("Could not configure the Odo database: {error}"))?;
    ensure_support_schema(&connection)?;
    Ok(connection)
}

pub fn ensure_support_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS mcp_activity (
           id INTEGER PRIMARY KEY AUTOINCREMENT,
           created_at INTEGER NOT NULL,
           transport TEXT NOT NULL,
           operation TEXT NOT NULL,
           target_id TEXT,
           outcome TEXT NOT NULL,
           detail TEXT NOT NULL DEFAULT ''
         );
         CREATE TABLE IF NOT EXISTS mcp_changes (
           id INTEGER PRIMARY KEY CHECK (id = 1),
           version INTEGER NOT NULL DEFAULT 0
         );
         INSERT OR IGNORE INTO mcp_changes (id, version) VALUES (1, 0);",
        )
        .map_err(|error| format!("Could not initialize MCP storage: {error}"))?;
    connection
        .execute(
            "DELETE FROM mcp_activity WHERE created_at < ?1",
            [now_epoch() - RETENTION_SECONDS],
        )
        .map_err(|error| format!("Could not prune MCP activity: {error}"))?;
    let folders_exist: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='folders')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("Could not inspect folder storage: {error}"))?;
    if folders_exist {
        let hierarchy_repaired: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM app_state WHERE key=?1)",
                [FOLDER_HIERARCHY_REPAIR_KEY],
                |row| row.get(0),
            )
            .map_err(|error| format!("Could not read folder repair state: {error}"))?;
        if !hierarchy_repaired {
            let repaired = repair_folder_hierarchy(connection)?;
            connection
                .execute(
                    "INSERT INTO app_state(key,value) VALUES(?1,?2)",
                    params![FOLDER_HIERARCHY_REPAIR_KEY, repaired.to_string()],
                )
                .map_err(|error| format!("Could not save folder repair state: {error}"))?;
        }
        let mut statement = connection
            .prepare("SELECT id, icon FROM folders WHERE id <> 'inbox'")
            .map_err(|error| format!("Could not inspect folder icons: {error}"))?;
        let invalid_ids = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|error| format!("Could not read folder icons: {error}"))?
            .filter_map(|row| match row {
                Ok((id, icon))
                    if normalize_folder_icon(icon.as_deref()) == DEFAULT_FOLDER_ICON
                        && icon.as_deref() != Some(DEFAULT_FOLDER_ICON) =>
                {
                    Some(Ok(id))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Could not decode folder icons: {error}"))?;
        drop(statement);
        for id in invalid_ids {
            connection
                .execute(
                    "UPDATE folders SET icon=?2 WHERE id=?1",
                    params![id, DEFAULT_FOLDER_ICON],
                )
                .map_err(|error| format!("Could not normalize a folder icon: {error}"))?;
        }
    }
    Ok(())
}

pub fn load_config(path: &Path) -> Result<McpConfig, String> {
    let connection = open(path)?;
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM app_state WHERE key = ?1",
            [CONFIG_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not load MCP settings: {error}"))?;
    match value {
        Some(value) => serde_json::from_str(&value)
            .map_err(|error| format!("The saved MCP settings are invalid: {error}")),
        None => {
            let config = McpConfig::default();
            save_config(path, &config)?;
            Ok(config)
        }
    }
}

pub fn save_config(path: &Path, config: &McpConfig) -> Result<(), String> {
    if config.host.trim().is_empty() {
        return Err("The bind address cannot be empty".into());
    }
    let connection = open(path)?;
    let value = serde_json::to_string(config)
        .map_err(|error| format!("Could not encode MCP settings: {error}"))?;
    connection
        .execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![CONFIG_KEY, value],
        )
        .map_err(|error| format!("Could not save MCP settings: {error}"))?;
    Ok(())
}

pub fn change_version(path: &Path) -> Result<i64, String> {
    open(path)?
        .query_row("SELECT version FROM mcp_changes WHERE id = 1", [], |row| {
            row.get(0)
        })
        .map_err(|error| format!("Could not read the MCP change version: {error}"))
}

fn bump_change(connection: &Connection) -> Result<(), String> {
    connection
        .execute(
            "UPDATE mcp_changes SET version = version + 1 WHERE id = 1",
            [],
        )
        .map_err(|error| format!("Could not publish the workspace change: {error}"))?;
    Ok(())
}

fn audit(
    connection: &Connection,
    transport: &str,
    operation: &str,
    target: Option<&str>,
    outcome: &str,
    detail: &str,
) {
    let _ = connection.execute(
        "INSERT INTO mcp_activity (created_at, transport, operation, target_id, outcome, detail)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![now_epoch(), transport, operation, target, outcome, detail],
    );
    let _ = connection.execute(
        "DELETE FROM mcp_activity WHERE created_at < ?1",
        [now_epoch() - RETENTION_SECONDS],
    );
}

fn tool_result(result: Result<Value, String>) -> Result<CallToolResult, McpError> {
    Ok(match result {
        Ok(value) => CallToolResult::structured(value),
        Err(message) => CallToolResult::structured_error(json!({ "error": message })),
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdArgs {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListNotesArgs {
    pub folder_id: Option<String>,
    pub status: Option<String>,
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub include_content: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNoteArgs {
    /// Exact note ID. Provide either id or title, but not both.
    pub id: Option<String>,
    /// Case-insensitive exact title. If multiple notes share the title, use id instead.
    pub title: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteArgs {
    pub folder_id: Option<String>,
    #[schemars(length(min = 1, max = 1000))]
    pub title: String,
    pub content: Option<String>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNoteArgs {
    pub id: String,
    pub expected_revision: i64,
    #[schemars(length(min = 1, max = 1000))]
    pub title: Option<String>,
    pub content: Option<String>,
    pub folder_id: Option<String>,
    pub status: Option<String>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModifyNoteArgs {
    pub id: String,
    pub expected_revision: i64,
    pub text: String,
    pub separator: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderArgs {
    #[schemars(length(min = 1, max = 1000))]
    pub name: String,
    /// Optional parent folder ID. Missing, empty, or unknown values create the folder at the root with parentId null.
    pub parent_id: Option<String>,
    /// Optional icon from the exact 50-value allowlist returned by folders_list.allowedIcons. Action icons are reserved.
    #[schemars(schema_with = "folder_icon_schema")]
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFolderArgs {
    pub id: String,
    #[schemars(length(min = 1, max = 1000))]
    pub name: Option<String>,
    pub parent_id: Option<String>,
    /// Set true to move the folder to the root. Cannot be combined with parentId.
    pub clear_parent: Option<bool>,
    /// Optional replacement icon from the exact 50-value allowlist returned by folders_list.allowedIcons. Action icons are reserved.
    #[schemars(schema_with = "folder_icon_schema")]
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksArgs {
    pub completed: Option<bool>,
    pub category_id: Option<String>,
    pub scheduled_from: Option<String>,
    pub scheduled_to: Option<String>,
    /// Case-insensitive substring filter over task text.
    pub query: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskArgs {
    #[schemars(length(min = 1))]
    pub text: String,
    pub category_id: Option<String>,
    /// One of low, medium, or high. Defaults to medium.
    #[schemars(schema_with = "task_priority_schema")]
    pub priority: Option<String>,
    /// Effort from 1 through 5. Out-of-range values are clamped to that range.
    #[schemars(range(min = 1, max = 5))]
    pub effort: Option<i64>,
    /// Optional six-digit hexadecimal color in #RRGGBB form.
    #[schemars(schema_with = "task_color_schema")]
    pub color: Option<String>,
    /// Optional timezone-qualified ISO-8601 timestamp.
    #[schemars(schema_with = "scheduled_start_schema")]
    pub scheduled_start: Option<String>,
    /// Positive duration in minutes. Values above 1,440 are clamped to one day.
    #[schemars(range(min = 1, max = 1440))]
    pub duration_minutes: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskArgs {
    pub id: String,
    pub text: Option<String>,
    pub completed: Option<bool>,
    pub category_id: Option<String>,
    pub priority: Option<String>,
    /// Effort from 1 through 5. Out-of-range values are clamped to that range.
    #[schemars(range(min = 1, max = 5))]
    pub effort: Option<i64>,
    pub color: Option<String>,
    pub scheduled_start: Option<String>,
    pub clear_schedule: Option<bool>,
    pub duration_minutes: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryArgs {
    pub id: Option<String>,
    pub name: String,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CategoryUpdateArgs {
    pub id: String,
    pub name: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JournalArgs {
    #[schemars(schema_with = "journal_date_schema")]
    pub date_key: String,
    pub content: Option<String>,
    pub append: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JournalDateArgs {
    #[schemars(schema_with = "journal_date_schema")]
    pub date_key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkdownTaskArgs {
    pub note_id: String,
    pub line: u32,
    pub completed: bool,
    pub expected_revision: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchArgs {
    pub query: String,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActivityArgs {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BatchOperation {
    /// Required discriminator. Set to notes.create, notes.update, notes.move, notes.status, notes.pin, tasks.create, tasks.complete, or tasks.delete.
    pub operation: String,
    pub id: Option<String>,
    /// Required for notes.update, notes.move, notes.status, and notes.pin; obtain it from notes_get, notes_list, or search.
    pub expected_revision: Option<i64>,
    pub folder_id: Option<String>,
    pub status: Option<String>,
    pub pinned: Option<bool>,
    #[schemars(length(min = 1, max = 1000))]
    pub title: Option<String>,
    pub content: Option<String>,
    pub text: Option<String>,
    pub completed: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BatchArgs {
    pub operations: Vec<BatchOperation>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PromptArgs {
    pub focus: Option<String>,
}

type ChangeNotifier = Arc<dyn Fn() + Send + Sync>;
type Subscribers = Arc<AsyncMutex<HashMap<String, Vec<Peer<RoleServer>>>>>;

#[derive(Clone)]
#[allow(dead_code)] // Router fields are consumed by rmcp's generated handler implementation.
pub struct OdoMcp {
    db_path: PathBuf,
    transport: String,
    notifier: Option<ChangeNotifier>,
    subscribers: Subscribers,
    tool_router: ToolRouter<Self>,
    prompt_router: PromptRouter<Self>,
}

impl OdoMcp {
    pub fn new(
        db_path: PathBuf,
        transport: impl Into<String>,
        notifier: Option<ChangeNotifier>,
    ) -> Self {
        Self {
            db_path,
            transport: transport.into(),
            notifier,
            subscribers: Arc::new(AsyncMutex::new(HashMap::new())),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
        }
    }

    fn connection(&self) -> Result<Connection, String> {
        open(&self.db_path)
    }

    async fn changed(&self, uris: &[String]) {
        if let Some(notifier) = &self.notifier {
            notifier();
        }
        let subscribers = self.subscribers.lock().await;
        for (subscribed_uri, peers) in subscribers.iter() {
            if uris
                .iter()
                .any(|uri| uri == subscribed_uri || uri.starts_with(subscribed_uri))
            {
                for peer in peers {
                    let _ = peer
                        .notify_resource_updated(ResourceUpdatedNotificationParam::new(
                            subscribed_uri.clone(),
                        ))
                        .await;
                }
            }
        }
    }

    fn note_json(connection: &Connection, id: &str) -> Result<Value, String> {
        connection.query_row(
            "SELECT id, folder_id, title, content, updated, status, pinned, revision FROM notes WHERE id = ?1",
            [id],
            |row| Ok(json!({
                "id": row.get::<_, String>(0)?, "folderId": row.get::<_, String>(1)?, "title": row.get::<_, String>(2)?,
                "content": row.get::<_, String>(3)?, "updated": row.get::<_, String>(4)?, "status": row.get::<_, String>(5)?,
                "pinned": row.get::<_, bool>(6)?, "revision": row.get::<_, i64>(7)?
            })),
        ).optional().map_err(|error| format!("Could not read note: {error}"))?
            .ok_or_else(|| format!("Note '{id}' was not found"))
    }

    fn note_json_by_title(connection: &Connection, title: &str) -> Result<Value, String> {
        let mut statement = connection
            .prepare(
                "SELECT id FROM notes WHERE title = ?1 COLLATE NOCASE ORDER BY updated DESC LIMIT 2",
            )
            .map_err(|error| format!("Could not find note by title: {error}"))?;
        let ids = statement
            .query_map([title], |row| row.get::<_, String>(0))
            .map_err(|error| format!("Could not search note titles: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("Could not decode note matches: {error}"))?;
        match ids.as_slice() {
            [] => Err(format!("No note with the exact title '{title}' was found")),
            [id] => Self::note_json(connection, id),
            ids => Err(format!(
                "Multiple notes have the title '{title}'. Call notes_get with one of these IDs instead: {}",
                ids.join(", ")
            )),
        }
    }

    fn task_json(connection: &Connection, id: &str) -> Result<Value, String> {
        connection
            .query_row(
                "SELECT id,text,completed,created,updated,category_id,priority,effort,color,scheduled_start,duration_minutes FROM todos WHERE id=?1",
                [id],
                |row| Ok(json!({
                    "id":row.get::<_,String>(0)?,"text":row.get::<_,String>(1)?,"completed":row.get::<_,bool>(2)?,
                    "created":row.get::<_,String>(3)?,"updated":row.get::<_,String>(4)?,"categoryId":row.get::<_,String>(5)?,
                    "priority":row.get::<_,String>(6)?,"effort":row.get::<_,i64>(7)?,"color":row.get::<_,String>(8)?,
                    "scheduledStart":row.get::<_,Option<String>>(9)?,"durationMinutes":row.get::<_,i64>(10)?
                })),
            )
            .optional()
            .map_err(|error| format!("Could not read task: {error}"))?
            .ok_or_else(|| format!("Task '{id}' was not found"))
    }

    fn folder_json(connection: &Connection, id: &str) -> Result<Value, String> {
        connection
            .query_row(
                "SELECT f.id,f.name,f.parent_id,f.icon,f.position,COUNT(n.id) FROM folders f LEFT JOIN notes n ON n.folder_id=f.id WHERE f.id=?1 GROUP BY f.id",
                [id],
                |row| {
                    let folder_id = row.get::<_, String>(0)?;
                    let stored_icon = row.get::<_, Option<String>>(3)?;
                    let icon = if folder_id == "inbox" {
                        stored_icon.unwrap_or_else(|| "ph-tray".into())
                    } else {
                        normalize_folder_icon(stored_icon.as_deref()).into()
                    };
                    Ok(json!({
                        "id":folder_id,"name":row.get::<_,String>(1)?,"parentId":row.get::<_,Option<String>>(2)?,
                        "icon":icon,"position":row.get::<_,i64>(4)?,"noteCount":row.get::<_,i64>(5)?
                    }))
                },
            )
            .optional()
            .map_err(|error| format!("Could not read folder: {error}"))?
            .ok_or_else(|| format!("Folder '{id}' was not found"))
    }

    fn category_json(connection: &Connection, id: &str) -> Result<Value, String> {
        connection
            .query_row(
                "SELECT id,name,color,icon,position FROM todo_categories WHERE id=?1",
                [id],
                |row| Ok(json!({
                    "id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"color":row.get::<_,String>(2)?,
                    "icon":row.get::<_,String>(3)?,"position":row.get::<_,i64>(4)?
                })),
            )
            .optional()
            .map_err(|error| format!("Could not read category: {error}"))?
            .ok_or_else(|| format!("Category '{id}' was not found"))
    }

    fn update_note_inner(&self, args: UpdateNoteArgs) -> Result<Value, String> {
        let connection = self.connection()?;
        let current = Self::note_json(&connection, &args.id)?;
        let current_revision = current["revision"].as_i64().unwrap_or_default();
        if current_revision != args.expected_revision {
            return Err(format!("Revision conflict: expected {}, but the note is now revision {}. Read it again before updating.", args.expected_revision, current_revision));
        }
        let title = args
            .title
            .unwrap_or_else(|| current["title"].as_str().unwrap_or_default().into());
        validate_note_title(&title)?;
        let content = args
            .content
            .unwrap_or_else(|| current["content"].as_str().unwrap_or_default().into());
        let folder_id = args
            .folder_id
            .unwrap_or_else(|| current["folderId"].as_str().unwrap_or("inbox").into());
        let status = args
            .status
            .unwrap_or_else(|| current["status"].as_str().unwrap_or("active").into());
        let pinned = args
            .pinned
            .unwrap_or_else(|| current["pinned"].as_bool().unwrap_or(false));
        if !matches!(status.as_str(), "active" | "archived" | "trash") {
            return Err("status must be active, archived, or trash".into());
        }
        let updated = now_iso();
        let count = connection.execute(
            "UPDATE notes SET folder_id=?2,title=?3,content=?4,updated=?5,status=?6,pinned=?7,revision=revision+1 WHERE id=?1 AND revision=?8",
            params![args.id, folder_id, title, content, updated, status, pinned, args.expected_revision],
        ).map_err(|error| format!("Could not update note: {error}"))?;
        if count == 0 {
            return Err("The note changed before the update could be saved".into());
        }
        bump_change(&connection)?;
        audit(
            &connection,
            &self.transport,
            "notes.update",
            Some(&args.id),
            "success",
            "content redacted",
        );
        Self::note_json(&connection, &args.id)
    }
}

#[tool_router]
impl OdoMcp {
    #[tool(
        description = "Return counts, configuration-safe metadata, and recent workspace state for the local Odo workspace"
    )]
    async fn workspace_summary(&self) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let count = |table: &str| -> Result<i64, String> {
                c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                    .map_err(|e| e.to_string())
            };
            Ok(
                json!({"folders":count("folders")?,"notes":count("notes")?,"tasks":count("todos")?,"categories":count("todo_categories")?,"journalEntries":count("journal_entries")?}),
            )
        })())
    }

    #[tool(
        description = "Create an immediate consistent SQLite backup in Odo's private local backups directory and return an opaque backupId without exposing the filesystem path"
    )]
    async fn workspace_backup(&self) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let connection = self.connection()?;
            let directory = self
                .db_path
                .parent()
                .ok_or_else(|| "The Odo data directory is unavailable".to_string())?
                .join("backups");
            fs::create_dir_all(&directory)
                .map_err(|e| format!("Could not create backup directory: {e}"))?;
            let backup_id = backup_identifier(now_epoch());
            let destination = directory.join(&backup_id);
            connection
                .backup("main", &destination, None)
                .map_err(|e| format!("Could not create backup: {e}"))?;
            audit(
                &connection,
                &self.transport,
                "workspace.backup",
                None,
                "success",
                "",
            );
            Ok(json!({"backupId":backup_id,"created":true}))
        })())
    }

    #[tool(
        description = "Search note titles/content, planner-task text, and journal content. query must contain at least one non-whitespace character. Results are globally ordered by most recent update and paginated with limit (default 50, max 200) and offset (default 0)."
    )]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let query = args.query.trim();
            if query.is_empty() {
                return Err("Query must not be empty".into());
            }
            let c = self.connection()?;
            let pattern = format!("%{query}%");
            let limit = args.limit.unwrap_or(50).min(200);
            let offset = args.offset.unwrap_or(0);
            if limit == 0 {
                let mut page = empty_paginated("results", offset);
                page["query"] = Value::String(query.to_string());
                return Ok(page);
            }
            let mut statement = c.prepare(
                "SELECT kind,id,label,excerpt,status,revision,completed,scheduled_start,date_key FROM (
                   SELECT 'note' AS kind,id,title AS label,substr(content,1,240) AS excerpt,status,revision,NULL AS completed,NULL AS scheduled_start,NULL AS date_key,updated AS sort_key
                   FROM notes WHERE title LIKE ?1 OR content LIKE ?1
                   UNION ALL
                   SELECT 'task',id,text,NULL,NULL,NULL,completed,scheduled_start,NULL,updated
                   FROM todos WHERE text LIKE ?1
                   UNION ALL
                   SELECT 'journal',id,date_key,substr(content,1,240),NULL,NULL,NULL,NULL,date_key,updated
                   FROM journal_entries WHERE content LIKE ?1 OR date_key LIKE ?1
                 ) ORDER BY sort_key DESC,id LIMIT ?2 OFFSET ?3"
            ).map_err(|error| error.to_string())?;
            let mut results = statement
                .query_map(params![pattern, i64::from(limit) + 1, i64::from(offset)], |row| {
                    let kind = row.get::<_, String>(0)?;
                    let id = row.get::<_, String>(1)?;
                    let label = row.get::<_, String>(2)?;
                    let excerpt = row.get::<_, Option<String>>(3)?;
                    Ok(match kind.as_str() {
                        "note" => json!({
                            "kind":"note","id":id,"title":label,"excerpt":excerpt.unwrap_or_default(),
                            "status":row.get::<_,Option<String>>(4)?,"revision":row.get::<_,Option<i64>>(5)?
                        }),
                        "task" => json!({
                            "kind":"task","id":id,"text":label,"completed":row.get::<_,Option<bool>>(6)?.unwrap_or(false),
                            "scheduledStart":row.get::<_,Option<String>>(7)?
                        }),
                        _ => json!({
                            "kind":"journal","id":id,"dateKey":row.get::<_,Option<String>>(8)?.unwrap_or(label),
                            "excerpt":excerpt.unwrap_or_default()
                        }),
                    })
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            let has_more = results.len() > limit as usize;
            results.truncate(limit as usize);
            Ok(json!({
                "query":query,"results":results,"limit":limit,"offset":offset,"hasMore":has_more,
                "nextOffset":if has_more { Some(offset + limit) } else { None }
            }))
        })())
    }

    #[tool(
        description = "Apply multiple backend mutations atomically. Each object in operations requires an operation field set to one of: notes.create, notes.update, notes.move, notes.status, notes.pin, tasks.create, tasks.complete, tasks.delete. notes.update, notes.move, notes.status, and notes.pin also require expectedRevision from the latest notes_get, notes_list, or search result; the whole batch is rejected on a missing or stale revision."
    )]
    async fn batch_apply(
        &self,
        Parameters(args): Parameters<BatchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            if args.operations.is_empty() {
                return Err("Batch must contain at least one operation".into());
            }
            if args.operations.len() > 100 {
                return Err("Batch is limited to 100 operations".into());
            }
            let mut c = self.connection()?;
            let tx = c.transaction().map_err(|e| e.to_string())?;
            let mut results = Vec::new();
            for (index, operation) in args.operations.into_iter().enumerate() {
                let op = operation.operation.as_str();
                let value = match op {
                    "notes.create" => {
                        let id = operation
                            .id
                            .unwrap_or_else(|| format!("note-{}", uuid::Uuid::new_v4()));
                        let title = operation.title.unwrap_or_else(|| "Untitled".into());
                        validate_note_title(&title)
                            .map_err(|error| format!("Batch item {index}: {error}"))?;
                        tx.execute("INSERT INTO notes(id,folder_id,title,content,updated,status,pinned,position,revision) VALUES(?1,?2,?3,?4,?5,'active',?6,(SELECT COALESCE(MAX(position),-1)+1 FROM notes),0)",params![id,operation.folder_id.unwrap_or_else(||"inbox".into()),title,operation.content.unwrap_or_default(),now_iso(),operation.pinned.unwrap_or(false)]).map_err(|e|format!("Batch item {index}: {e}"))?;
                        json!({"id":id,"revision":0})
                    }
                    "notes.update" | "notes.move" | "notes.status" | "notes.pin" => {
                        let id = operation
                            .id
                            .ok_or_else(|| format!("Batch item {index}: note id is required"))?;
                        let expected = operation.expected_revision.ok_or_else(|| {
                            format!("Batch item {index}: expectedRevision is required")
                        })?;
                        let current:(String,String,String,String,bool)=tx.query_row("SELECT folder_id,title,content,status,pinned FROM notes WHERE id=?1 AND revision=?2",params![id,expected],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(|e|e.to_string())?.ok_or_else(||format!("Batch item {index}: note missing or revision conflict"))?;
                        let status = operation.status.unwrap_or(current.3);
                        if !matches!(status.as_str(), "active" | "archived" | "trash") {
                            return Err(format!("Batch item {index}: invalid note status"));
                        }
                        let title = operation.title.unwrap_or(current.1);
                        validate_note_title(&title)
                            .map_err(|error| format!("Batch item {index}: {error}"))?;
                        tx.execute("UPDATE notes SET folder_id=?2,title=?3,content=?4,status=?5,pinned=?6,updated=?7,revision=revision+1 WHERE id=?1 AND revision=?8",params![id,operation.folder_id.unwrap_or(current.0),title,operation.content.unwrap_or(current.2),status,operation.pinned.unwrap_or(current.4),now_iso(),expected]).map_err(|e|e.to_string())?;
                        json!({"id":id,"revision":expected+1})
                    }
                    "tasks.create" => {
                        let id = operation
                            .id
                            .unwrap_or_else(|| format!("todo-{}", uuid::Uuid::new_v4()));
                        let stamp = now_iso();
                        tx.execute("INSERT INTO todos(id,text,completed,created,updated,position,category_id,priority,effort,color,scheduled_start,duration_minutes) VALUES(?1,?2,?3,?4,?4,(SELECT COALESCE(MAX(position),-1)+1 FROM todos),'inbox','medium',2,'',NULL,30)",params![id,operation.text.unwrap_or_else(||"Untitled task".into()),operation.completed.unwrap_or(false),stamp]).map_err(|e|format!("Batch item {index}: {e}"))?;
                        json!({"id":id})
                    }
                    "tasks.complete" => {
                        let id = operation
                            .id
                            .ok_or_else(|| format!("Batch item {index}: task id is required"))?;
                        if tx
                            .execute(
                                "UPDATE todos SET completed=?2,updated=?3 WHERE id=?1",
                                params![id, operation.completed.unwrap_or(true), now_iso()],
                            )
                            .map_err(|e| e.to_string())?
                            == 0
                        {
                            return Err(format!("Batch item {index}: task not found"));
                        }
                        json!({"id":id})
                    }
                    "tasks.delete" => {
                        let id = operation
                            .id
                            .ok_or_else(|| format!("Batch item {index}: task id is required"))?;
                        if tx
                            .execute("DELETE FROM todos WHERE id=?1", [&id])
                            .map_err(|e| e.to_string())?
                            == 0
                        {
                            return Err(format!("Batch item {index}: task not found"));
                        }
                        json!({"id":id,"deleted":true})
                    }
                    _ => return Err(format!("Batch item {index}: unsupported operation '{op}'")),
                };
                results.push(json!({"index":index,"operation":op,"result":value}));
            }
            tx.execute("UPDATE mcp_changes SET version=version+1 WHERE id=1", [])
                .map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            audit(
                &c,
                &self.transport,
                "batch.apply",
                None,
                "success",
                &format!("{} operations", results.len()),
            );
            Ok(json!({"applied":results.len(),"results":results}))
        })();
        if result.is_ok() {
            self.changed(&[
                "odo://workspace".into(),
                "odo://notes".into(),
                "odo://tasks".into(),
            ])
            .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "List the complete nested folder model with parent IDs, validated icons, direct note counts, and allowedIcons: the exact 50 folder icon values accepted by folders_create and folders_update. Action icons used by Odo navigation or mutations are intentionally excluded."
    )]
    async fn folders_list(&self) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let mut s=c.prepare("SELECT f.id,f.name,f.parent_id,f.icon,f.position,COUNT(n.id) FROM folders f LEFT JOIN notes n ON n.folder_id=f.id GROUP BY f.id ORDER BY f.position").map_err(|e|e.to_string())?;
            let rows=s.query_map([],|r|{
                let id=r.get::<_,String>(0)?;
                let stored=r.get::<_,Option<String>>(3)?;
                let icon=if id=="inbox"{stored.unwrap_or_else(||"ph-tray".into())}else{normalize_folder_icon(stored.as_deref()).into()};
                Ok(json!({"id":id,"name":r.get::<_,String>(1)?,"parentId":r.get::<_,Option<String>>(2)?,"icon":icon,"position":r.get::<_,i64>(4)?,"noteCount":r.get::<_,i64>(5)?}))
            }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            Ok(json!({"folders":rows,"allowedIcons":ALLOWED_FOLDER_ICONS.as_slice()}))
        })())
    }

    #[tool(
        description = "Create a folder, optionally nested under an existing parent folder, and return the complete created folder object. A missing, empty, or unknown parentId is normalized to the workspace root. icon defaults to ph-folder and must be one of the exact 50 values from folders_list.allowedIcons; archive, trash, delete, add, edit, and other action icons are reserved and rejected."
    )]
    async fn folders_create(
        &self,
        Parameters(args): Parameters<CreateFolderArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            let name = validate_folder_name(&args.name)?;
            let parent_id = normalize_folder_parent_for_create(&c, args.parent_id)?;
            let icon = validate_folder_icon(args.icon)?;
            let id = format!("folder-{}", uuid::Uuid::new_v4());
            c.execute("INSERT INTO folders(id,name,parent_id,is_open,icon,position) VALUES(?1,?2,?3,1,?4,(SELECT COALESCE(MAX(position),-1)+1 FROM folders))",params![id,name,parent_id,icon]).map_err(|e|format!("Could not create folder: {e}"))?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "folders.create",
                Some(&id),
                "success",
                "",
            );
            Self::folder_json(&c, &id)
        })();
        if result.is_ok() {
            self.changed(&["odo://folders".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Rename, move, or change the icon of an existing folder and return the complete updated folder object. parentId must reference an existing folder and is rejected if it would create a circular hierarchy; set clearParent=true to move a folder to the root. A supplied icon must be one of the exact 50 values from folders_list.allowedIcons; action icons are reserved and rejected."
    )]
    async fn folders_update(
        &self,
        Parameters(args): Parameters<UpdateFolderArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            let current: (String, Option<String>, Option<String>) = c
                .query_row(
                    "SELECT name,parent_id,icon FROM folders WHERE id=?1",
                    [&args.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Folder not found".to_string())?;
            if args.clear_parent.unwrap_or(false) && args.parent_id.is_some() {
                return Err("clearParent cannot be combined with parentId".into());
            }
            let name = match args.name {
                Some(name) => validate_folder_name(&name)?.to_string(),
                None => current.0,
            };
            let parent = if args.clear_parent.unwrap_or(false) {
                None
            } else {
                args.parent_id.or(current.1)
            };
            validate_folder_parent(&c, Some(&args.id), parent.as_deref())?;
            let icon = match args.icon {
                Some(icon) => validate_folder_icon(Some(icon))?,
                None if args.id == "inbox" => current.2.unwrap_or_else(|| "ph-tray".into()),
                None => normalize_folder_icon(current.2.as_deref()).into(),
            };
            c.execute(
                "UPDATE folders SET name=?2,parent_id=?3,icon=?4 WHERE id=?1",
                params![args.id, name, parent, icon],
            )
            .map_err(|e| e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "folders.update",
                Some(&args.id),
                "success",
                "",
            );
            Self::folder_json(&c, &args.id)
        })();
        if result.is_ok() {
            self.changed(&["odo://folders".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Delete a folder tree with cycle-safe traversal, moving every contained note to Inbox and Trash so the operation remains recoverable even if legacy data contains circular folder references"
    )]
    async fn folders_delete(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            if args.id == "inbox" {
                return Err("Inbox cannot be deleted".into());
            }
            let mut c = self.connection()?;
            let deleted = delete_folder_tree(&mut c, &args.id)?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "folders.delete",
                Some(&args.id),
                "success",
                "contents moved to Trash",
            );
            Ok(json!({"deletedFolders":deleted,"notesMovedToTrash":true}))
        })();
        if result.is_ok() {
            self.changed(&[
                "odo://folders".into(),
                "odo://notes".into(),
                "odo://workspace".into(),
            ])
            .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "List notes with optional folder, status, and text filters. Content is excluded by default and returned as null; set includeContent=true to include full Markdown. Results are ordered by most recently updated and paginated with limit (default 100, max 500) and offset (default 0)."
    )]
    async fn notes_list(
        &self,
        Parameters(args): Parameters<ListNotesArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let limit = args.limit.unwrap_or(100).min(500);
            let offset = args.offset.unwrap_or(0);
            if limit == 0 {
                return Ok(empty_paginated("notes", offset));
            }
            let mut sql="SELECT id,folder_id,title,content,updated,status,pinned,revision FROM notes WHERE 1=1".to_string();
            let mut values: Vec<String> = vec![];
            if let Some(v) = args.folder_id {
                sql.push_str(" AND folder_id=?");
                values.push(v)
            }
            if let Some(v) = args.status {
                sql.push_str(" AND status=?");
                values.push(v)
            }
            if let Some(v) = args.query {
                sql.push_str(" AND (title LIKE ? OR content LIKE ?)");
                let p = format!("%{v}%");
                values.push(p.clone());
                values.push(p)
            }
            sql.push_str(" ORDER BY updated DESC LIMIT ? OFFSET ?");
            values.push((limit + 1).to_string());
            values.push(offset.to_string());
            let mut s = c.prepare(&sql).map_err(|e| e.to_string())?;
            let mut rows=s.query_map(rusqlite::params_from_iter(values.iter()),|r|{let content:String=r.get(3)?;Ok(json!({"id":r.get::<_,String>(0)?,"folderId":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"content":if args.include_content.unwrap_or(false){Value::String(content.clone())}else{Value::Null},"excerpt":content.chars().take(240).collect::<String>(),"updated":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,"pinned":r.get::<_,bool>(6)?,"revision":r.get::<_,i64>(7)?}))}).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            let has_more = rows.len() > limit as usize;
            rows.truncate(limit as usize);
            Ok(
                json!({"notes":rows,"limit":limit,"offset":offset,"hasMore":has_more,"nextOffset":if has_more{Some(offset+limit)}else{None}}),
            )
        })())
    }

    #[tool(
        description = "Read a complete note including Markdown content and the revision required for safe updates. Provide exactly one lookup field: id or title. id first checks an exact note ID, then falls back to a case-insensitive exact title for clients that put a known title in the id field. title performs the same title lookup. Duplicate titles are rejected with matching IDs so the caller can disambiguate."
    )]
    async fn notes_get(
        &self,
        Parameters(args): Parameters<GetNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let connection = self.connection()?;
            match (args.id, args.title) {
                (Some(id), None) if !id.trim().is_empty() => {
                    match Self::note_json(&connection, &id) {
                        Ok(note) => Ok(note),
                        Err(error) if error == format!("Note '{id}' was not found") => {
                            Self::note_json_by_title(&connection, id.trim())
                        }
                        Err(error) => Err(error),
                    }
                }
                (None, Some(title)) if !title.trim().is_empty() => {
                    Self::note_json_by_title(&connection, title.trim())
                }
                (Some(_), Some(_)) => Err("Provide either id or title, not both".into()),
                _ => Err("Provide a non-empty id or title".into()),
            }
        })())
    }

    #[tool(
        description = "Create a Markdown note in a folder and return the complete note object; defaults to Inbox. title is required, cannot be blank, and must contain at most 1,000 characters."
    )]
    async fn notes_create(
        &self,
        Parameters(args): Parameters<CreateNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            validate_note_title(&args.title)?;
            let id = format!("note-{}", uuid::Uuid::new_v4());
            let folder = args.folder_id.unwrap_or_else(|| "inbox".into());
            let updated = now_iso();
            c.execute("INSERT INTO notes(id,folder_id,title,content,updated,status,pinned,position,revision) VALUES(?1,?2,?3,?4,?5,'active',?6,(SELECT COALESCE(MAX(position),-1)+1 FROM notes),0)",params![id,folder,args.title,args.content.unwrap_or_default(),updated,args.pinned.unwrap_or(false)]).map_err(|e|format!("Could not create note: {e}"))?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "notes.create",
                Some(&id),
                "success",
                "content redacted",
            );
            Self::note_json(&c, &id)
        })();
        if result.is_ok() {
            self.changed(&["odo://notes".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Safely update note fields using expectedRevision and return the complete updated note; rejects stale writes instead of overwriting newer edits. If title is supplied, it cannot be blank and must contain at most 1,000 characters."
    )]
    async fn notes_update(
        &self,
        Parameters(args): Parameters<UpdateNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let id = args.id.clone();
        let result = self.update_note_inner(args);
        if result.is_ok() {
            self.changed(&[
                format!("odo://notes/{id}"),
                "odo://notes".into(),
                "odo://workspace".into(),
            ])
            .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Append Markdown to a note atomically using expectedRevision. The separator is inserted after the existing content and before the new text; it defaults to two newlines."
    )]
    async fn notes_append(
        &self,
        Parameters(args): Parameters<ModifyNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let c = self.connection();
        let result = c.and_then(|c| Self::note_json(&c, &args.id)).and_then(|n| {
            let sep = args.separator.unwrap_or_else(|| "\n\n".into());
            let content = format!(
                "{}{}{}",
                n["content"].as_str().unwrap_or_default(),
                sep,
                args.text
            );
            self.update_note_inner(UpdateNoteArgs {
                id: args.id.clone(),
                expected_revision: args.expected_revision,
                title: None,
                content: Some(content),
                folder_id: None,
                status: None,
                pinned: None,
            })
        });
        if result.is_ok() {
            self.changed(&[format!("odo://notes/{}", args.id), "odo://notes".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Prepend Markdown to a note atomically using expectedRevision. The separator is inserted after the new text and before the existing content; it defaults to two newlines."
    )]
    async fn notes_prepend(
        &self,
        Parameters(args): Parameters<ModifyNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let c = self.connection();
        let result = c.and_then(|c| Self::note_json(&c, &args.id)).and_then(|n| {
            let sep = args.separator.unwrap_or_else(|| "\n\n".into());
            let content = format!(
                "{}{}{}",
                args.text,
                sep,
                n["content"].as_str().unwrap_or_default()
            );
            self.update_note_inner(UpdateNoteArgs {
                id: args.id.clone(),
                expected_revision: args.expected_revision,
                title: None,
                content: Some(content),
                folder_id: None,
                status: None,
                pinned: None,
            })
        });
        if result.is_ok() {
            self.changed(&[format!("odo://notes/{}", args.id), "odo://notes".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Permanently delete a trashed note when the separate permanent-delete permission is enabled"
    )]
    async fn notes_delete_permanently(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let config = load_config(&self.db_path)?;
            if !config.permanent_delete_enabled {
                return Err("Permanent deletion is disabled in Odo Settings".into());
            }
            let c = self.connection()?;
            let count = c
                .execute(
                    "DELETE FROM notes WHERE id=?1 AND status='trash'",
                    [&args.id],
                )
                .map_err(|e| e.to_string())?;
            if count == 0 {
                return Err("Only an existing trashed note can be permanently deleted".into());
            }
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "notes.delete_permanently",
                Some(&args.id),
                "success",
                "",
            );
            Ok(json!({"deleted":true,"id":args.id}))
        })();
        if result.is_ok() {
            self.changed(&["odo://notes".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "List Markdown checkbox tasks embedded in notes, including note ID, line number, text, completion, and note revision"
    )]
    async fn markdown_tasks_list(&self) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let mut s = c
                .prepare("SELECT id,title,content,revision FROM notes WHERE status='active'")
                .map_err(|e| e.to_string())?;
            let mut tasks = vec![];
            for row in s
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|e| e.to_string())?
            {
                let (id, title, content, revision) = row.map_err(|e| e.to_string())?;
                for (index, line) in content.lines().enumerate() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with("- [ ] ") || trimmed.to_lowercase().starts_with("- [x] ")
                    {
                        tasks.push(json!({"noteId":id,"noteTitle":title,"line":index,"text":trimmed.get(6..).unwrap_or_default(),"completed":trimmed.to_lowercase().starts_with("- [x]"),"revision":revision}));
                    }
                }
            }
            Ok(json!({"tasks":tasks}))
        })())
    }

    #[tool(
        description = "Complete or reopen one Markdown checkbox by zero-based line number with note revision protection"
    )]
    async fn markdown_tasks_toggle(
        &self,
        Parameters(args): Parameters<MarkdownTaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = self
            .connection()
            .and_then(|c| Self::note_json(&c, &args.note_id))
            .and_then(|n| {
                let mut lines = n["content"]
                    .as_str()
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let line = lines
                    .get_mut(args.line as usize)
                    .ok_or_else(|| "Markdown task line is out of range".to_string())?;
                let trimmed = line.trim_start();
                if !(trimmed.starts_with("- [ ] ") || trimmed.to_lowercase().starts_with("- [x] "))
                {
                    return Err("The selected line is not a Markdown checkbox".into());
                }
                let indent = &line[..line.len() - trimmed.len()];
                *line = format!(
                    "{}- [{}] {}",
                    indent,
                    if args.completed { "x" } else { " " },
                    trimmed.get(6..).unwrap_or_default()
                );
                self.update_note_inner(UpdateNoteArgs {
                    id: args.note_id.clone(),
                    expected_revision: args.expected_revision,
                    title: None,
                    content: Some(lines.join("\n")),
                    folder_id: None,
                    status: None,
                    pinned: None,
                })
            });
        if result.is_ok() {
            self.changed(&[
                format!("odo://notes/{}", args.note_id),
                "odo://notes".into(),
            ])
            .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "List planner tasks with completion, category, scheduling, and case-insensitive text-query filters. Results preserve planner order and are paginated with limit (default 100, max 500) and offset (default 0)."
    )]
    async fn tasks_list(
        &self,
        Parameters(args): Parameters<ListTasksArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let limit = args.limit.unwrap_or(100).min(500);
            let offset = args.offset.unwrap_or(0);
            if limit == 0 {
                return Ok(empty_paginated("tasks", offset));
            }
            let mut sql="SELECT id,text,completed,created,updated,category_id,priority,effort,color,scheduled_start,duration_minutes FROM todos WHERE 1=1".to_string();
            let mut values: Vec<String> = vec![];
            if let Some(completed) = args.completed {
                sql.push_str(" AND completed=?");
                values.push(if completed { "1" } else { "0" }.into());
            }
            if let Some(category) = args.category_id {
                sql.push_str(" AND category_id=?");
                values.push(category);
            }
            if let Some(from) = args.scheduled_from {
                sql.push_str(" AND scheduled_start IS NOT NULL AND scheduled_start>=?");
                values.push(from);
            }
            if let Some(to) = args.scheduled_to {
                sql.push_str(" AND scheduled_start IS NOT NULL AND scheduled_start<=?");
                values.push(to);
            }
            if let Some(query) = args.query.filter(|query| !query.trim().is_empty()) {
                sql.push_str(" AND text LIKE ?");
                values.push(format!("%{}%", query.trim()));
            }
            sql.push_str(" ORDER BY position LIMIT ? OFFSET ?");
            values.push((limit + 1).to_string());
            values.push(offset.to_string());
            let mut statement = c.prepare(&sql).map_err(|error| error.to_string())?;
            let mut rows=statement.query_map(rusqlite::params_from_iter(values.iter()),|r|Ok(json!({"id":r.get::<_,String>(0)?,"text":r.get::<_,String>(1)?,"completed":r.get::<_,bool>(2)?,"created":r.get::<_,String>(3)?,"updated":r.get::<_,String>(4)?,"categoryId":r.get::<_,String>(5)?,"priority":r.get::<_,String>(6)?,"effort":r.get::<_,i64>(7)?,"color":r.get::<_,String>(8)?,"scheduledStart":r.get::<_,Option<String>>(9)?,"durationMinutes":r.get::<_,i64>(10)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            let has_more = rows.len() > limit as usize;
            rows.truncate(limit as usize);
            Ok(
                json!({"tasks":rows,"limit":limit,"offset":offset,"hasMore":has_more,"nextOffset":if has_more{Some(offset+limit)}else{None}}),
            )
        })())
    }

    #[tool(
        description = "Create a planner task and return the complete object. Text must not be blank; priority is low, medium, or high; explicit color uses #RRGGBB; scheduledStart is a timezone-qualified ISO-8601 timestamp; effort is clamped to 1–5; durationMinutes must be positive and is clamped to 1,440."
    )]
    async fn tasks_create(
        &self,
        Parameters(args): Parameters<CreateTaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            validate_task_text(&args.text)?;
            let priority = validate_task_priority(args.priority)?;
            let color = validate_task_color(args.color)?;
            let scheduled_start = validate_scheduled_start(args.scheduled_start)?;
            let duration_minutes = normalize_task_duration(args.duration_minutes)?;
            let c = self.connection()?;
            let id = format!("todo-{}", uuid::Uuid::new_v4());
            let stamp = now_iso();
            c.execute("INSERT INTO todos(id,text,completed,created,updated,position,category_id,priority,effort,color,scheduled_start,duration_minutes) VALUES(?1,?2,0,?3,?3,(SELECT COALESCE(MAX(position),-1)+1 FROM todos),?4,?5,?6,?7,?8,?9)",params![id,args.text,stamp,args.category_id.unwrap_or_else(||"inbox".into()),priority,args.effort.unwrap_or(2).clamp(1,5),color,scheduled_start,duration_minutes]).map_err(|e|e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "tasks.create",
                Some(&id),
                "success",
                "",
            );
            Self::task_json(&c, &id)
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Update any planner task field, including completion and scheduling; returns the complete updated task object"
    )]
    async fn tasks_update(
        &self,
        Parameters(args): Parameters<UpdateTaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            let current:(String,bool,String,String,i64,String,Option<String>,i64)=c.query_row("SELECT text,completed,category_id,priority,effort,color,scheduled_start,duration_minutes FROM todos WHERE id=?1",[&args.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|e|e.to_string())?.ok_or_else(||"Task not found".to_string())?;
            let scheduled = if args.clear_schedule.unwrap_or(false) {
                None
            } else {
                args.scheduled_start.or(current.6)
            };
            c.execute("UPDATE todos SET text=?2,completed=?3,updated=?4,category_id=?5,priority=?6,effort=?7,color=?8,scheduled_start=?9,duration_minutes=?10 WHERE id=?1",params![args.id,args.text.unwrap_or(current.0),args.completed.unwrap_or(current.1),now_iso(),args.category_id.unwrap_or(current.2),args.priority.unwrap_or(current.3),args.effort.unwrap_or(current.4).clamp(1,5),args.color.unwrap_or(current.5),scheduled,args.duration_minutes.unwrap_or(current.7).max(30)]).map_err(|e|e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "tasks.update",
                Some(&args.id),
                "success",
                "",
            );
            Self::task_json(&c, &args.id)
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(description = "Delete a planner task")]
    async fn tasks_delete(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            if c.execute("DELETE FROM todos WHERE id=?1", [&args.id])
                .map_err(|e| e.to_string())?
                == 0
            {
                return Err("Task not found".into());
            }
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "tasks.delete",
                Some(&args.id),
                "success",
                "",
            );
            Ok(json!({"deleted":true}))
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(description = "List planner task categories")]
    async fn categories_list(&self) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let mut s = c
                .prepare(
                    "SELECT id,name,color,icon,position FROM todo_categories ORDER BY position",
                )
                .map_err(|e| e.to_string())?;
            let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"color":r.get::<_,String>(2)?,"icon":r.get::<_,String>(3)?,"position":r.get::<_,i64>(4)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            Ok(json!({"categories":rows}))
        })())
    }

    #[tool(
        description = "Create a planner task category and return the complete created category object"
    )]
    async fn categories_create(
        &self,
        Parameters(args): Parameters<CategoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            let id = args
                .id
                .unwrap_or_else(|| format!("category-{}", uuid::Uuid::new_v4()));
            c.execute("INSERT INTO todo_categories(id,name,color,icon,position) VALUES(?1,?2,?3,?4,(SELECT COALESCE(MAX(position),-1)+1 FROM todo_categories))",params![id,args.name,args.color.unwrap_or_else(||"#7b8e7c".into()),args.icon.unwrap_or_default()]).map_err(|e|e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "categories.create",
                Some(&id),
                "success",
                "",
            );
            Self::category_json(&c, &id)
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into()]).await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Rename or recolor a planner task category and return the complete updated category object"
    )]
    async fn categories_update(
        &self,
        Parameters(args): Parameters<CategoryUpdateArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            let current: (String, String, String) = c
                .query_row(
                    "SELECT name,color,icon FROM todo_categories WHERE id=?1",
                    [&args.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Category not found".to_string())?;
            c.execute(
                "UPDATE todo_categories SET name=?2,color=?3,icon=?4 WHERE id=?1",
                params![
                    args.id,
                    args.name.unwrap_or(current.0),
                    args.color.unwrap_or(current.1),
                    args.icon.unwrap_or(current.2)
                ],
            )
            .map_err(|e| e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "categories.update",
                Some(&args.id),
                "success",
                "",
            );
            Self::category_json(&c, &args.id)
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into()]).await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Delete a planner category and move its tasks to Inbox; the Inbox category itself cannot be deleted"
    )]
    async fn categories_delete(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            if args.id == "inbox" {
                return Err("Inbox category cannot be deleted".into());
            }
            let mut c = self.connection()?;
            let tx = c.transaction().map_err(|e| e.to_string())?;
            tx.execute("UPDATE todos SET category_id='inbox',updated=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE category_id=?1",[&args.id]).map_err(|e|e.to_string())?;
            if tx
                .execute("DELETE FROM todo_categories WHERE id=?1", [&args.id])
                .map_err(|e| e.to_string())?
                == 0
            {
                return Err("Category not found".into());
            }
            tx.commit().map_err(|e| e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "categories.delete",
                Some(&args.id),
                "success",
                "tasks moved to Inbox",
            );
            Ok(json!({"deleted":true,"tasksMovedTo":"inbox"}))
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(description = "List journal days with compact excerpts and timestamps")]
    async fn journal_list(&self) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let mut s=c.prepare("SELECT id,date_key,substr(content,1,240),created,updated FROM journal_entries ORDER BY date_key DESC").map_err(|e|e.to_string())?;
            let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"dateKey":r.get::<_,String>(1)?,"excerpt":r.get::<_,String>(2)?,"created":r.get::<_,String>(3)?,"updated":r.get::<_,String>(4)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            Ok(json!({"journalEntries":rows}))
        })())
    }

    #[tool(description = "Read a journal entry for a YYYY-MM-DD date")]
    async fn journal_get(
        &self,
        Parameters(args): Parameters<JournalDateArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            c.query_row("SELECT id,date_key,content,created,updated FROM journal_entries WHERE date_key=?1",[args.date_key],|r|Ok(json!({"id":r.get::<_,String>(0)?,"dateKey":r.get::<_,String>(1)?,"content":r.get::<_,String>(2)?,"created":r.get::<_,String>(3)?,"updated":r.get::<_,String>(4)?}))).optional().map_err(|e|e.to_string())?.ok_or_else(||"Journal entry not found".into())
        })())
    }

    #[tool(description = "Create, replace, or append to a YYYY-MM-DD journal entry")]
    async fn journal_upsert(
        &self,
        Parameters(args): Parameters<JournalArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            validate_journal_date_key(&args.date_key)?;
            let c = self.connection()?;
            let existing: Option<(String, String, String)> = c
                .query_row(
                    "SELECT id,content,created FROM journal_entries WHERE date_key=?1",
                    [&args.date_key],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let stamp = now_iso();
            let input = args.content.unwrap_or_default();
            let (id, content, created) = match existing {
                Some((id, old, created)) => (
                    id,
                    if args.append.unwrap_or(false) && !old.is_empty() {
                        format!("{old}\n\n{input}")
                    } else {
                        input
                    },
                    created,
                ),
                None => (
                    format!("journal-{}", uuid::Uuid::new_v4()),
                    input,
                    stamp.clone(),
                ),
            };
            c.execute("INSERT INTO journal_entries(id,date_key,content,created,updated,position) VALUES(?1,?2,?3,?4,?5,(SELECT COALESCE(MAX(position),-1)+1 FROM journal_entries)) ON CONFLICT(date_key) DO UPDATE SET content=excluded.content,updated=excluded.updated",params![id,args.date_key,content,created,stamp]).map_err(|e|e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "journal.upsert",
                Some(&id),
                "success",
                "content redacted",
            );
            Ok(json!({"id":id,"dateKey":args.date_key}))
        })();
        if result.is_ok() {
            self.changed(&[
                format!("odo://journal/{}", args.date_key),
                "odo://journal".into(),
            ])
            .await;
        }
        tool_result(result)
    }

    #[tool(description = "Delete a journal entry for a YYYY-MM-DD date")]
    async fn journal_delete(
        &self,
        Parameters(args): Parameters<JournalDateArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            if c.execute(
                "DELETE FROM journal_entries WHERE date_key=?1",
                [&args.date_key],
            )
            .map_err(|e| e.to_string())?
                == 0
            {
                return Err("Journal entry not found".into());
            }
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "journal.delete",
                Some(&args.date_key),
                "success",
                "",
            );
            Ok(json!({"deleted":true,"dateKey":args.date_key}))
        })();
        if result.is_ok() {
            self.changed(&[
                format!("odo://journal/{}", args.date_key),
                "odo://journal".into(),
                "odo://workspace".into(),
            ])
            .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "List the redacted MCP activity audit retained for the previous 60 days, newest first. Content fields are redacted in the audit log for privacy. Paginate with limit (default 100, max 1000) and offset (default 0)."
    )]
    async fn activity_list(
        &self,
        Parameters(args): Parameters<ActivityArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let limit = args.limit.unwrap_or(100).min(1000);
            let offset = args.offset.unwrap_or(0);
            if limit == 0 {
                let mut page = empty_paginated("activity", offset);
                page["retentionDays"] = json!(60);
                return Ok(page);
            }
            let mut s=c.prepare("SELECT id,created_at,transport,operation,target_id,outcome,detail FROM mcp_activity ORDER BY id DESC LIMIT ?1 OFFSET ?2").map_err(|e|e.to_string())?;
            let mut rows=s.query_map(params![limit+1,offset],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"createdAt":r.get::<_,i64>(1)?,"transport":r.get::<_,String>(2)?,"operation":r.get::<_,String>(3)?,"targetId":r.get::<_,Option<String>>(4)?,"outcome":r.get::<_,String>(5)?,"detail":r.get::<_,String>(6)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            let has_more = rows.len() > limit as usize;
            rows.truncate(limit as usize);
            Ok(
                json!({"retentionDays":60,"activity":rows,"limit":limit,"offset":offset,"hasMore":has_more,"nextOffset":if has_more{Some(offset+limit)}else{None}}),
            )
        })())
    }
}

#[prompt_router]
impl OdoMcp {
    #[prompt(
        name = "daily_planning",
        description = "Build a realistic prioritized day plan from today's Odo tasks and journal."
    )]
    async fn daily_planning(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Review my Odo planner tasks and today's journal. Build a realistic daily plan with priorities, time blocks, and a small finish line. Focus: {}",args.focus.unwrap_or_else(||"balanced progress".into())))])
    }
    #[prompt(
        name = "inbox_triage",
        description = "Organize Inbox notes and unscheduled tasks into folders, next actions, and safe archival suggestions."
    )]
    async fn inbox_triage(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Review Odo Inbox notes and unscheduled planner tasks. Propose folders, task conversions, archival, and next actions without permanently deleting anything. Focus: {}",args.focus.unwrap_or_else(||"clarity".into())))])
    }
    #[prompt(
        name = "weekly_review",
        description = "Review the week's notes, tasks, and journal entries for wins, lessons, unfinished work, and next priorities."
    )]
    async fn weekly_review(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Use this week's Odo notes, completed and open tasks, and journal entries to run a weekly review: wins, unfinished work, lessons, and next-week priorities. Focus: {}",args.focus.unwrap_or_else(||"sustainable momentum".into())))])
    }
    #[prompt(
        name = "meeting_note_processing",
        description = "Turn an Odo meeting note into decisions, owners, deadlines, follow-ups, and planner tasks."
    )]
    async fn meeting_note_processing(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Process the relevant Odo meeting note into decisions, owners, deadlines, follow-ups, and planner tasks. Preserve the original note. Focus: {}",args.focus.unwrap_or_else(||"clear accountability".into())))])
    }
    #[prompt(
        name = "overdue_task_review",
        description = "Triage overdue and unscheduled planner tasks into complete, reschedule, delegate, split, or delete recommendations."
    )]
    async fn overdue_task_review(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Review overdue and unscheduled Odo planner tasks. Recommend complete, reschedule, delegate, split, or delete actions and explain tradeoffs. Focus: {}",args.focus.unwrap_or_else(||"honest prioritization".into())))])
    }
}

#[tool_handler]
#[prompt_handler]
impl ServerHandler for OdoMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().enable_prompts().enable_resources().enable_resources_subscribe().build())
            .with_server_info(Implementation::new("odo-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("Odo is a local notes, planner-task, folder, and journal workspace. Read current revisions before replacing note content. Permanent note deletion is separately controlled in Odo Settings. MCP tools never control or navigate the frontend.")
    }

    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                Resource::new("odo://workspace", "Odo workspace summary").with_description("Counts for folders, notes, planner tasks, and journal entries in the local workspace.").with_mime_type("text/markdown"),
                Resource::new("odo://folders", "Odo folder tree").with_description("The complete nested folder index with parent IDs and validated icons.").with_mime_type("text/markdown"),
                Resource::new("odo://notes", "Odo notes index").with_description("A compact index of notes with folder, status, pin, timestamp, and revision metadata. Read a complete note through the advertised odo://notes/{id} resource template.").with_mime_type("text/markdown"),
                Resource::new("odo://tasks", "Odo planner tasks").with_description("All planner tasks with completion, category, priority, effort, and scheduling metadata.").with_mime_type("text/markdown"),
                Resource::new("odo://journal", "Odo journal index").with_description("A newest-first index of journal days with compact excerpts and timestamps. Read a complete day through the advertised odo://journal/{date} resource template.").with_mime_type("text/markdown"),
                Resource::new("odo://activity", "Odo MCP activity (60 days)").with_description("The redacted MCP operation audit retained for the previous 60 days.").with_mime_type("text/markdown"),
            ],
            ..Default::default()
        })
    }

    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![
                ResourceTemplate::new("odo://notes/{id}", "Odo note").with_description("A complete note, including Markdown content and revision metadata, addressed by note ID.").with_mime_type("text/markdown"),
                ResourceTemplate::new("odo://journal/{date}", "Odo journal day").with_description("A complete journal entry addressed by its YYYY-MM-DD date key.").with_mime_type("text/markdown"),
            ],
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri.clone();
        let result: Result<Value, String> = (|| {
            if uri == "odo://workspace" {
                let c = self.connection()?;
                let count = |table: &str| {
                    c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .map_err(|e| e.to_string())
                };
                return Ok(
                    json!({"folders":count("folders")?,"notes":count("notes")?,"tasks":count("todos")?,"journalEntries":count("journal_entries")?}),
                );
            }
            if uri == "odo://folders" {
                let c = self.connection()?;
                let mut s = c
                    .prepare("SELECT id,name,parent_id,icon FROM folders ORDER BY position")
                    .map_err(|e| e.to_string())?;
                let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"parentId":r.get::<_,Option<String>>(2)?,"icon":r.get::<_,Option<String>>(3)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
                return Ok(json!(rows));
            }
            if uri == "odo://notes" {
                let c = self.connection()?;
                let mut s=c.prepare("SELECT id,folder_id,title,updated,status,pinned,revision FROM notes ORDER BY updated DESC").map_err(|e|e.to_string())?;
                let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"folderId":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"updated":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"pinned":r.get::<_,bool>(5)?,"revision":r.get::<_,i64>(6)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
                return Ok(json!(rows));
            }
            if uri == "odo://tasks" {
                let c = self.connection()?;
                let mut s=c.prepare("SELECT id,text,completed,category_id,priority,effort,scheduled_start,duration_minutes FROM todos ORDER BY position").map_err(|e|e.to_string())?;
                let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"text":r.get::<_,String>(1)?,"completed":r.get::<_,bool>(2)?,"categoryId":r.get::<_,String>(3)?,"priority":r.get::<_,String>(4)?,"effort":r.get::<_,i64>(5)?,"scheduledStart":r.get::<_,Option<String>>(6)?,"durationMinutes":r.get::<_,i64>(7)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
                return Ok(json!(rows));
            }
            if uri == "odo://journal" {
                let c = self.connection()?;
                let mut s=c.prepare("SELECT id,date_key,substr(content,1,240),created,updated FROM journal_entries ORDER BY date_key DESC").map_err(|e|e.to_string())?;
                let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"dateKey":r.get::<_,String>(1)?,"excerpt":r.get::<_,String>(2)?,"created":r.get::<_,String>(3)?,"updated":r.get::<_,String>(4)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
                return Ok(json!(rows));
            }
            if uri == "odo://activity" {
                let c = self.connection()?;
                let mut s=c.prepare("SELECT created_at,transport,operation,target_id,outcome,detail FROM mcp_activity ORDER BY id DESC LIMIT 500").map_err(|e|e.to_string())?;
                let rows=s.query_map([],|r|Ok(json!({"createdAt":r.get::<_,i64>(0)?,"transport":r.get::<_,String>(1)?,"operation":r.get::<_,String>(2)?,"targetId":r.get::<_,Option<String>>(3)?,"outcome":r.get::<_,String>(4)?,"detail":r.get::<_,String>(5)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
                return Ok(json!({"retentionDays":60,"activity":rows}));
            }
            if let Some(id) = uri.strip_prefix("odo://notes/") {
                return self.connection().and_then(|c| Self::note_json(&c, id));
            }
            if let Some(date) = uri.strip_prefix("odo://journal/") {
                let c = self.connection()?;
                return c.query_row("SELECT id,date_key,content,created,updated FROM journal_entries WHERE date_key=?1",[date],|r|Ok(json!({"id":r.get::<_,String>(0)?,"dateKey":r.get::<_,String>(1)?,"content":r.get::<_,String>(2)?,"created":r.get::<_,String>(3)?,"updated":r.get::<_,String>(4)?}))).optional().map_err(|e|e.to_string())?.ok_or_else(||"Journal entry not found".into());
            }
            Err(format!(
                "Use the matching list tool or a specific resource URI for {uri}"
            ))
        })();
        match result {
            Ok(value) => {
                let body =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    format!("```json\n{body}\n```"),
                    uri,
                )
                .with_mime_type("text/markdown")]))
            }
            Err(message) => Err(McpError::resource_not_found(
                message,
                Some(json!({"uri":uri})),
            )),
        }
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.subscribers
            .lock()
            .await
            .entry(request.uri)
            .or_default()
            .push(context.peer);
        Ok(())
    }
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.subscribers.lock().await.remove(&request.uri);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn folder_icon_allowlist_has_50_unique_non_action_icons() {
        assert_eq!(ALLOWED_FOLDER_ICONS.len(), 50);
        assert_eq!(
            ALLOWED_FOLDER_ICONS
                .iter()
                .copied()
                .collect::<HashSet<_>>()
                .len(),
            50
        );
        for reserved in [
            "ph-archive",
            "ph-archive-tray",
            "ph-trash",
            "ph-trash-simple",
            "ph-tray",
            "ph-plus",
            "ph-pencil",
        ] {
            assert!(!ALLOWED_FOLDER_ICONS.contains(&reserved));
            assert!(validate_folder_icon(Some(reserved.into())).is_err());
        }
        for icon in ALLOWED_FOLDER_ICONS {
            assert_eq!(validate_folder_icon(Some(icon.into())).unwrap(), icon);
        }
    }

    #[test]
    fn support_schema_normalizes_existing_invalid_folder_icons() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE folders (id TEXT PRIMARY KEY, parent_id TEXT, icon TEXT, position INTEGER);
                 INSERT INTO folders(id,parent_id,icon,position) VALUES ('inbox',NULL,'ph-tray',0);
                 INSERT INTO folders(id,parent_id,icon,position) VALUES ('bad',NULL,'ph-trash',1);
                 INSERT INTO folders(id,parent_id,icon,position) VALUES ('good',NULL,'ph-book',2);",
            )
            .unwrap();
        ensure_support_schema(&connection).unwrap();
        let bad: String = connection
            .query_row("SELECT icon FROM folders WHERE id='bad'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let good: String = connection
            .query_row("SELECT icon FROM folders WHERE id='good'", [], |row| {
                row.get(0)
            })
            .unwrap();
        let inbox: String = connection
            .query_row("SELECT icon FROM folders WHERE id='inbox'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(bad, DEFAULT_FOLDER_ICON);
        assert_eq!(good, "ph-book");
        assert_eq!(inbox, "ph-tray");
    }

    #[test]
    fn support_schema_repairs_cycles_and_orphaned_parents_once() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE folders (id TEXT PRIMARY KEY, parent_id TEXT, icon TEXT, position INTEGER);
                 INSERT INTO folders VALUES ('inbox',NULL,'ph-tray',0);
                 INSERT INTO folders VALUES ('a','c','ph-folder',1);
                 INSERT INTO folders VALUES ('b','a','ph-folder',2);
                 INSERT INTO folders VALUES ('c','b','ph-folder',3);
                 INSERT INTO folders VALUES ('orphan','missing','ph-folder',4);",
            )
            .unwrap();
        ensure_support_schema(&connection).unwrap();
        let parent = |id: &str| {
            connection
                .query_row("SELECT parent_id FROM folders WHERE id=?1", [id], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .unwrap()
        };
        assert_eq!(parent("a"), None);
        assert_eq!(parent("b").as_deref(), Some("a"));
        assert_eq!(parent("c").as_deref(), Some("b"));
        assert_eq!(parent("orphan"), None);
        let repair_marker: String = connection
            .query_row(
                "SELECT value FROM app_state WHERE key=?1",
                [FOLDER_HIERARCHY_REPAIR_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(repair_marker, "2");
    }

    #[test]
    fn folder_parent_validation_rejects_cycles_and_missing_parents() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE folders (id TEXT PRIMARY KEY, parent_id TEXT);
                 INSERT INTO folders VALUES ('inbox',NULL);
                 INSERT INTO folders VALUES ('a',NULL);
                 INSERT INTO folders VALUES ('b','a');",
            )
            .unwrap();
        assert!(validate_folder_parent(&connection, Some("a"), Some("b"))
            .unwrap_err()
            .contains("circular"));
        assert!(
            validate_folder_parent(&connection, Some("a"), Some("missing"))
                .unwrap_err()
                .contains("not found")
        );
        assert!(validate_folder_parent(&connection, Some("a"), Some("a"))
            .unwrap_err()
            .contains("own parent"));
        validate_folder_parent(&connection, Some("b"), None).unwrap();
    }

    #[test]
    fn cyclic_folder_delete_finishes_and_releases_the_write_lock() {
        let path =
            std::env::temp_dir().join(format!("odo-cycle-delete-{}.sqlite3", uuid::Uuid::new_v4()));
        let mut connection = Connection::open(&path).unwrap();
        connection
            .busy_timeout(std::time::Duration::from_millis(500))
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE folders (id TEXT PRIMARY KEY, parent_id TEXT);
                 CREATE TABLE notes (id TEXT PRIMARY KEY, folder_id TEXT, status TEXT, updated TEXT, revision INTEGER);
                 INSERT INTO folders VALUES ('a','c');
                 INSERT INTO folders VALUES ('b','a');
                 INSERT INTO folders VALUES ('c','b');
                 INSERT INTO notes VALUES ('note','b','active','',0);",
            )
            .unwrap();
        let started = std::time::Instant::now();
        assert_eq!(delete_folder_tree(&mut connection, "a").unwrap(), 3);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        let note: (String, String, i64) = connection
            .query_row(
                "SELECT folder_id,status,revision FROM notes WHERE id='note'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(note, ("inbox".into(), "trash".into(), 1));
        let second = Connection::open(&path).unwrap();
        second
            .busy_timeout(std::time::Duration::from_millis(500))
            .unwrap();
        second
            .execute_batch("BEGIN IMMEDIATE; CREATE TABLE lock_probe(id INTEGER); COMMIT;")
            .unwrap();
        drop(second);
        drop(connection);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn journal_read_schema_only_accepts_date_key() {
        let schema = rmcp::schemars::schema_for!(JournalDateArgs);
        let value = serde_json::to_value(schema).unwrap();
        let properties = value["properties"].as_object().unwrap();
        assert_eq!(properties.len(), 1);
        assert!(properties.contains_key("dateKey"));
    }

    #[test]
    fn folder_create_schema_exposes_the_exact_icon_enum() {
        let schema = rmcp::schemars::schema_for!(CreateFolderArgs);
        let value = serde_json::to_value(schema).unwrap();
        let icons = value["properties"]["icon"]["enum"].as_array().unwrap();
        assert_eq!(icons.len(), 50);
        assert!(icons.contains(&Value::String("ph-folder".into())));
        assert!(!icons.contains(&Value::String("ph-trash".into())));
    }

    #[test]
    fn note_titles_must_be_non_blank_and_at_most_one_thousand_characters() {
        assert_eq!(
            validate_note_title("").unwrap_err(),
            "Note title cannot be empty"
        );
        assert_eq!(
            validate_note_title(" \n\t").unwrap_err(),
            "Note title cannot be empty"
        );
        validate_note_title(&"a".repeat(MAX_NOTE_TITLE_CHARS)).unwrap();
        assert_eq!(
            validate_note_title(&"🦀".repeat(MAX_NOTE_TITLE_CHARS + 1)).unwrap_err(),
            "Note title cannot exceed 1000 characters"
        );
    }

    #[test]
    fn folder_names_must_be_non_blank_and_at_most_one_thousand_characters() {
        assert_eq!(
            validate_folder_name(" \n\t").unwrap_err(),
            "Folder name cannot be empty"
        );
        assert_eq!(validate_folder_name("  Projects  ").unwrap(), "Projects");
        validate_folder_name(&"a".repeat(MAX_FOLDER_NAME_CHARS)).unwrap();
        assert_eq!(
            validate_folder_name(&"🦀".repeat(MAX_FOLDER_NAME_CHARS + 1)).unwrap_err(),
            "Folder name cannot exceed 1000 characters"
        );
    }

    #[test]
    fn journal_dates_require_exact_real_calendar_dates() {
        for invalid in ["", "07/15/2026", "2026-13-45", "2026-02-29", "2026-7-15"] {
            assert_eq!(
                validate_journal_date_key(invalid).unwrap_err(),
                "Journal dateKey must be a valid date in YYYY-MM-DD format"
            );
        }
        validate_journal_date_key("2026-07-15").unwrap();
        validate_journal_date_key("2028-02-29").unwrap();
    }

    #[test]
    fn task_creation_values_are_validated_and_bounded() {
        assert_eq!(
            validate_task_text(" \n").unwrap_err(),
            "Task text cannot be empty"
        );
        assert_eq!(validate_task_priority(None).unwrap(), "medium");
        assert_eq!(
            validate_task_priority(Some("super-urgent".into())).unwrap_err(),
            "Task priority must be one of: low, medium, high"
        );
        assert_eq!(
            validate_task_color(Some("#A1b2C3".into())).unwrap(),
            "#A1b2C3"
        );
        assert!(validate_task_color(Some("not-a-color".into())).is_err());
        assert!(validate_scheduled_start(Some("not-a-date".into())).is_err());
        assert!(validate_scheduled_start(Some("2026-07-15T19:41:00".into())).is_err());
        assert_eq!(
            validate_scheduled_start(Some("2026-07-15T19:41:00-05:00".into())).unwrap(),
            Some("2026-07-15T19:41:00-05:00".into())
        );
        assert!(normalize_task_duration(Some(-30)).is_err());
        assert!(normalize_task_duration(Some(0)).is_err());
        assert_eq!(normalize_task_duration(None).unwrap(), 30);
        assert_eq!(
            normalize_task_duration(Some(999_999)).unwrap(),
            MAX_TASK_DURATION_MINUTES
        );
    }

    #[test]
    fn mutation_schemas_advertise_validation_contracts() {
        let task_schema =
            serde_json::to_value(rmcp::schemars::schema_for!(CreateTaskArgs)).unwrap();
        let task = &task_schema["properties"];
        assert_eq!(task["text"]["minLength"], 1);
        assert_eq!(task["priority"]["enum"], json!(["low", "medium", "high"]));
        assert_eq!(task["effort"]["minimum"], 1);
        assert_eq!(task["effort"]["maximum"], 5);
        assert_eq!(task["color"]["pattern"], r"^#[0-9A-Fa-f]{6}$");
        assert_eq!(task["scheduledStart"]["format"], "date-time");
        assert_eq!(task["durationMinutes"]["minimum"], 1);
        assert_eq!(task["durationMinutes"]["maximum"], 1440);

        let journal_schema =
            serde_json::to_value(rmcp::schemars::schema_for!(JournalArgs)).unwrap();
        assert_eq!(
            journal_schema["properties"]["dateKey"]["pattern"],
            r"^\d{4}-\d{2}-\d{2}$"
        );

        let folder_schema =
            serde_json::to_value(rmcp::schemars::schema_for!(CreateFolderArgs)).unwrap();
        assert_eq!(folder_schema["properties"]["name"]["maxLength"], 1000);
        assert!(folder_schema["properties"]["parentId"]["description"]
            .as_str()
            .unwrap()
            .contains("unknown values create the folder at the root"));
    }

    #[test]
    fn zero_limit_page_is_empty_and_terminal_at_the_requested_offset() {
        let page = empty_paginated("notes", 37);
        assert_eq!(page["notes"], json!([]));
        assert_eq!(page["limit"], 0);
        assert_eq!(page["offset"], 37);
        assert_eq!(page["hasMore"], false);
        assert!(page["nextOffset"].is_null());
    }

    #[test]
    fn missing_folder_parent_is_normalized_to_root_on_create() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE folders (id TEXT PRIMARY KEY, parent_id TEXT);
                 INSERT INTO folders VALUES ('inbox',NULL);
                 INSERT INTO folders VALUES ('projects',NULL);",
            )
            .unwrap();
        assert_eq!(
            normalize_folder_parent_for_create(&connection, Some("projects".into())).unwrap(),
            Some("projects".into())
        );
        assert_eq!(
            normalize_folder_parent_for_create(&connection, Some("missing".into())).unwrap(),
            None
        );
        assert_eq!(
            normalize_folder_parent_for_create(&connection, Some("  ".into())).unwrap(),
            None
        );
    }

    #[test]
    fn backup_identifier_does_not_expose_a_filesystem_path() {
        let identifier = backup_identifier(1_784_158_320);
        assert_eq!(identifier, "workspace-mcp-1784158320.sqlite3");
        assert!(!identifier.contains('/'));
        assert!(!identifier.contains('\\'));
        assert!(!identifier.contains("Users"));
    }
}

pub async fn run_stdio(path: PathBuf) -> Result<(), String> {
    let config = load_config(&path)?;
    if !config.enabled {
        return Err(
            "The Odo MCP Server is disabled. Enable it from Odo Settings or the menu bar.".into(),
        );
    }
    use rmcp::{transport::stdio, ServiceExt};
    let service = OdoMcp::new(path, "stdio", None)
        .serve(stdio())
        .await
        .map_err(|e| format!("Could not start stdio MCP: {e}"))?;
    service
        .waiting()
        .await
        .map_err(|e| format!("The stdio MCP session failed: {e}"))?;
    Ok(())
}

#[derive(Clone)]
struct SecurityState {
    token: Option<String>,
    allowed_host: String,
}

async fn security_middleware(
    axum::extract::State(state): axum::extract::State<SecurityState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    if let Some(origin) = request
        .headers()
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let allowed = origin == "null"
            || origin.starts_with("tauri://")
            || origin.starts_with("https://tauri.localhost")
            || origin.contains("localhost")
            || origin.contains("127.0.0.1")
            || origin.contains(&state.allowed_host);
        if !allowed {
            return (StatusCode::FORBIDDEN, "Origin is not allowed").into_response();
        }
    }
    if let Some(token) = &state.token {
        let expected = format!("Bearer {token}");
        if request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            != Some(expected.as_str())
        {
            return (StatusCode::UNAUTHORIZED, "Missing or invalid Odo MCP token").into_response();
        }
    }
    next.run(request).await
}

pub async fn run_http(
    path: PathBuf,
    config: McpConfig,
    cancel: tokio_util::sync::CancellationToken,
    notifier: Option<ChangeNotifier>,
    status: Arc<Mutex<McpStatus>>,
) -> Result<(), String> {
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    let prototype = OdoMcp::new(path, "http", notifier);
    let service = StreamableHttpService::new(
        move || Ok(prototype.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default()
            .with_sse_retry(None)
            .with_cancellation_token(cancel.child_token()),
    );
    let security = SecurityState {
        token: config.auth_enabled.then_some(config.token.clone()),
        allowed_host: config.host.clone(),
    };
    let router = axum::Router::new().nest_service("/mcp", service).layer(
        axum::middleware::from_fn_with_state(security, security_middleware),
    );
    let mut listener = None;
    for candidate in config.port..=config.port.saturating_add(20) {
        if let Ok(bound) = tokio::net::TcpListener::bind((config.host.as_str(), candidate)).await {
            listener = Some((bound, candidate));
            break;
        }
    }
    let (listener, port) = match listener {
        Some(found) => found,
        None => (
            tokio::net::TcpListener::bind((config.host.as_str(), 0))
                .await
                .map_err(|e| format!("Could not bind MCP server to {}: {e}", config.host))?,
            0,
        ),
    };
    let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let endpoint = format!(
        "http://{}:{}/mcp",
        config.host,
        if port == 0 { actual_port } else { port }
    );
    *status.lock().expect("lock MCP status") = McpStatus {
        running: true,
        endpoint: Some(endpoint),
        error: None,
    };
    axum::serve(listener, router)
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
        .map_err(|e| format!("MCP HTTP server failed: {e}"))?;
    *status.lock().expect("lock MCP status") = McpStatus::default();
    Ok(())
}
