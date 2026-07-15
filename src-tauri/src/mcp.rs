use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use rmcp::{
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    prompt, prompt_handler, prompt_router,
    schemars::JsonSchema,
    service::{Peer, RequestContext},
    tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex as AsyncMutex;

const CONFIG_KEY: &str = "mcp_config_v1";
const RETENTION_SECONDS: i64 = 60 * 24 * 60 * 60;

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub running: bool,
    pub endpoint: Option<String>,
    pub error: Option<String>,
}

impl Default for McpStatus {
    fn default() -> Self {
        Self {
            running: false,
            endpoint: None,
            error: None,
        }
    }
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
    pub include_content: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteArgs {
    pub folder_id: Option<String>,
    pub title: String,
    pub content: Option<String>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNoteArgs {
    pub id: String,
    pub expected_revision: i64,
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
    pub name: String,
    pub parent_id: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFolderArgs {
    pub id: String,
    pub name: Option<String>,
    pub parent_id: Option<String>,
    pub icon: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTasksArgs {
    pub completed: Option<bool>,
    pub category_id: Option<String>,
    pub scheduled_from: Option<String>,
    pub scheduled_to: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskArgs {
    pub text: String,
    pub category_id: Option<String>,
    pub priority: Option<String>,
    pub effort: Option<i64>,
    pub color: Option<String>,
    pub scheduled_start: Option<String>,
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
    pub date_key: String,
    pub content: Option<String>,
    pub append: Option<bool>,
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
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActivityArgs {
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BatchOperation {
    pub operation: String,
    pub id: Option<String>,
    pub expected_revision: Option<i64>,
    pub folder_id: Option<String>,
    pub status: Option<String>,
    pub pinned: Option<bool>,
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
        description = "Create an immediate consistent SQLite backup in Odo's local backups directory"
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
            let destination = directory.join(format!("workspace-mcp-{}.sqlite3", now_epoch()));
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
            Ok(json!({"path":destination,"created":true}))
        })())
    }

    #[tool(
        description = "Search note titles/content, planner tasks, and journal entries; returns compact matches"
    )]
    async fn search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let pattern = format!("%{}%", args.query);
            let limit = args.limit.unwrap_or(50).min(200) as i64;
            let mut results = Vec::new();
            let mut s=c.prepare("SELECT id,title,substr(content,1,240),status,revision FROM notes WHERE title LIKE ?1 OR content LIKE ?1 ORDER BY updated DESC LIMIT ?2").map_err(|e|e.to_string())?;
            for row in s.query_map(params![pattern,limit],|r|Ok(json!({"kind":"note","id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"excerpt":r.get::<_,String>(2)?,"status":r.get::<_,String>(3)?,"revision":r.get::<_,i64>(4)?}))).map_err(|e|e.to_string())? { results.push(row.map_err(|e|e.to_string())?); }
            let mut s=c.prepare("SELECT id,text,completed,scheduled_start FROM todos WHERE text LIKE ?1 ORDER BY updated DESC LIMIT ?2").map_err(|e|e.to_string())?;
            for row in s.query_map(params![pattern,limit],|r|Ok(json!({"kind":"task","id":r.get::<_,String>(0)?,"text":r.get::<_,String>(1)?,"completed":r.get::<_,bool>(2)?,"scheduledStart":r.get::<_,Option<String>>(3)?}))).map_err(|e|e.to_string())? { results.push(row.map_err(|e|e.to_string())?); }
            let mut s=c.prepare("SELECT id,date_key,substr(content,1,240) FROM journal_entries WHERE content LIKE ?1 ORDER BY date_key DESC LIMIT ?2").map_err(|e|e.to_string())?;
            for row in s.query_map(params![pattern,limit],|r|Ok(json!({"kind":"journal","id":r.get::<_,String>(0)?,"dateKey":r.get::<_,String>(1)?,"excerpt":r.get::<_,String>(2)?}))).map_err(|e|e.to_string())? { results.push(row.map_err(|e|e.to_string())?); }
            Ok(json!({"query":args.query,"results":results}))
        })())
    }

    #[tool(
        description = "Apply multiple backend mutations atomically. Supported operations: notes.create, notes.update, notes.move, notes.status, notes.pin, tasks.create, tasks.complete, tasks.delete"
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
                        tx.execute("INSERT INTO notes(id,folder_id,title,content,updated,status,pinned,position,revision) VALUES(?1,?2,?3,?4,?5,'active',?6,(SELECT COALESCE(MAX(position),-1)+1 FROM notes),0)",params![id,operation.folder_id.unwrap_or_else(||"inbox".into()),operation.title.unwrap_or_else(||"Untitled".into()),operation.content.unwrap_or_default(),now_iso(),operation.pinned.unwrap_or(false)]).map_err(|e|format!("Batch item {index}: {e}"))?;
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
                        tx.execute("UPDATE notes SET folder_id=?2,title=?3,content=?4,status=?5,pinned=?6,updated=?7,revision=revision+1 WHERE id=?1 AND revision=?8",params![id,operation.folder_id.unwrap_or(current.0),operation.title.unwrap_or(current.1),operation.content.unwrap_or(current.2),status,operation.pinned.unwrap_or(current.4),now_iso(),expected]).map_err(|e|e.to_string())?;
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
        description = "List the complete nested folder model with parent IDs, icons, and direct note counts"
    )]
    async fn folders_list(&self) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let mut s=c.prepare("SELECT f.id,f.name,f.parent_id,f.icon,f.position,COUNT(n.id) FROM folders f LEFT JOIN notes n ON n.folder_id=f.id GROUP BY f.id ORDER BY f.position").map_err(|e|e.to_string())?;
            let rows=s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"name":r.get::<_,String>(1)?,"parentId":r.get::<_,Option<String>>(2)?,"icon":r.get::<_,Option<String>>(3)?,"position":r.get::<_,i64>(4)?,"noteCount":r.get::<_,i64>(5)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            Ok(json!({"folders":rows}))
        })())
    }

    #[tool(description = "Create a folder, optionally nested under another folder")]
    async fn folders_create(
        &self,
        Parameters(args): Parameters<CreateFolderArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            if args.name.trim().is_empty() {
                return Err("Folder name cannot be empty".into());
            }
            let id = format!("folder-{}", uuid::Uuid::new_v4());
            c.execute("INSERT INTO folders(id,name,parent_id,is_open,icon,position) VALUES(?1,?2,?3,1,?4,(SELECT COALESCE(MAX(position),-1)+1 FROM folders))",params![id,args.name.trim(),args.parent_id,args.icon]).map_err(|e|format!("Could not create folder: {e}"))?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "folders.create",
                Some(&id),
                "success",
                "",
            );
            Ok(json!({"id":id}))
        })();
        if result.is_ok() {
            self.changed(&["odo://folders".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(description = "Rename, move, or change the icon of an existing folder")]
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
            let name = args.name.unwrap_or(current.0);
            let parent = args.parent_id.or(current.1);
            let icon = args.icon.or(current.2);
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
            Ok(json!({"id":args.id}))
        })();
        if result.is_ok() {
            self.changed(&["odo://folders".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(
        description = "Delete a folder tree, moving every contained note to Inbox and Trash so the operation remains recoverable"
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
            let tx = c.transaction().map_err(|e| e.to_string())?;
            tx.execute("WITH RECURSIVE tree(id) AS (SELECT ?1 UNION ALL SELECT f.id FROM folders f JOIN tree t ON f.parent_id=t.id) UPDATE notes SET folder_id='inbox',status='trash',updated=strftime('%Y-%m-%dT%H:%M:%fZ','now'),revision=revision+1 WHERE folder_id IN tree",[&args.id]).map_err(|e|e.to_string())?;
            let deleted=tx.execute("WITH RECURSIVE tree(id) AS (SELECT ?1 UNION ALL SELECT f.id FROM folders f JOIN tree t ON f.parent_id=t.id) DELETE FROM folders WHERE id IN tree",[&args.id]).map_err(|e|e.to_string())?;
            if deleted == 0 {
                return Err("Folder not found".into());
            }
            tx.commit().map_err(|e| e.to_string())?;
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
        description = "List notes with optional folder, status, text query, content inclusion, and bounded result count"
    )]
    async fn notes_list(
        &self,
        Parameters(args): Parameters<ListNotesArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
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
            sql.push_str(" ORDER BY updated DESC LIMIT ?");
            values.push(args.limit.unwrap_or(100).min(500).to_string());
            let mut s = c.prepare(&sql).map_err(|e| e.to_string())?;
            let rows=s.query_map(rusqlite::params_from_iter(values.iter()),|r|{let content:String=r.get(3)?;Ok(json!({"id":r.get::<_,String>(0)?,"folderId":r.get::<_,String>(1)?,"title":r.get::<_,String>(2)?,"content":if args.include_content.unwrap_or(false){Value::String(content.clone())}else{Value::Null},"excerpt":content.chars().take(240).collect::<String>(),"updated":r.get::<_,String>(4)?,"status":r.get::<_,String>(5)?,"pinned":r.get::<_,bool>(6)?,"revision":r.get::<_,i64>(7)?}))}).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            Ok(json!({"notes":rows}))
        })())
    }

    #[tool(
        description = "Read a complete note including Markdown content and its revision required for safe updates"
    )]
    async fn notes_get(
        &self,
        Parameters(args): Parameters<IdArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result(
            self.connection()
                .and_then(|c| Self::note_json(&c, &args.id)),
        )
    }

    #[tool(description = "Create a Markdown note in a folder; defaults to Inbox")]
    async fn notes_create(
        &self,
        Parameters(args): Parameters<CreateNoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
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
        description = "Safely update note fields using expectedRevision; rejects stale writes instead of overwriting newer edits"
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

    #[tool(description = "Append Markdown to a note atomically using expectedRevision")]
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

    #[tool(description = "Prepend Markdown to a note atomically using expectedRevision")]
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
        description = "List planner tasks with completion, category, priority, effort, color, scheduling, and duration filters"
    )]
    async fn tasks_list(
        &self,
        Parameters(args): Parameters<ListTasksArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let mut s=c.prepare("SELECT id,text,completed,created,updated,category_id,priority,effort,color,scheduled_start,duration_minutes FROM todos ORDER BY position").map_err(|e|e.to_string())?;
            let mut rows = vec![];
            for row in s.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"text":r.get::<_,String>(1)?,"completed":r.get::<_,bool>(2)?,"created":r.get::<_,String>(3)?,"updated":r.get::<_,String>(4)?,"categoryId":r.get::<_,String>(5)?,"priority":r.get::<_,String>(6)?,"effort":r.get::<_,i64>(7)?,"color":r.get::<_,String>(8)?,"scheduledStart":r.get::<_,Option<String>>(9)?,"durationMinutes":r.get::<_,i64>(10)?}))).map_err(|e|e.to_string())?{let v=row.map_err(|e|e.to_string())?;if args.completed.is_some_and(|x|v["completed"]!=x)||args.category_id.as_ref().is_some_and(|x|v["categoryId"]!=*x)||args.scheduled_from.as_ref().is_some_and(|x|v["scheduledStart"].as_str().is_none_or(|s|s<x.as_str()))||args.scheduled_to.as_ref().is_some_and(|x|v["scheduledStart"].as_str().is_none_or(|s|s>x.as_str())){continue}rows.push(v)}
            Ok(json!({"tasks":rows}))
        })())
    }

    #[tool(
        description = "Create a planner task with category, priority, effort, color, optional schedule, and duration"
    )]
    async fn tasks_create(
        &self,
        Parameters(args): Parameters<CreateTaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = (|| {
            let c = self.connection()?;
            let id = format!("todo-{}", uuid::Uuid::new_v4());
            let stamp = now_iso();
            c.execute("INSERT INTO todos(id,text,completed,created,updated,position,category_id,priority,effort,color,scheduled_start,duration_minutes) VALUES(?1,?2,0,?3,?3,(SELECT COALESCE(MAX(position),-1)+1 FROM todos),?4,?5,?6,?7,?8,?9)",params![id,args.text,stamp,args.category_id.unwrap_or_else(||"inbox".into()),args.priority.unwrap_or_else(||"medium".into()),args.effort.unwrap_or(2).clamp(1,5),args.color.unwrap_or_default(),args.scheduled_start,args.duration_minutes.unwrap_or(30).max(30)]).map_err(|e|e.to_string())?;
            bump_change(&c)?;
            audit(
                &c,
                &self.transport,
                "tasks.create",
                Some(&id),
                "success",
                "",
            );
            Ok(json!({"id":id}))
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into(), "odo://workspace".into()])
                .await;
        }
        tool_result(result)
    }

    #[tool(description = "Update any planner task field, including completion and scheduling")]
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
            Ok(json!({"id":args.id}))
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

    #[tool(description = "Create a planner task category")]
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
            Ok(json!({"id":id}))
        })();
        if result.is_ok() {
            self.changed(&["odo://tasks".into()]).await;
        }
        tool_result(result)
    }

    #[tool(description = "Rename or recolor a planner task category")]
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
            Ok(json!({"id":args.id}))
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
        Parameters(args): Parameters<JournalArgs>,
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
        Parameters(args): Parameters<JournalArgs>,
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

    #[tool(description = "List the redacted MCP activity audit retained for the previous 60 days")]
    async fn activity_list(
        &self,
        Parameters(args): Parameters<ActivityArgs>,
    ) -> Result<CallToolResult, McpError> {
        tool_result((|| {
            let c = self.connection()?;
            let mut s=c.prepare("SELECT id,created_at,transport,operation,target_id,outcome,detail FROM mcp_activity ORDER BY id DESC LIMIT ?1").map_err(|e|e.to_string())?;
            let rows=s.query_map([args.limit.unwrap_or(100).min(1000)],|r|Ok(json!({"id":r.get::<_,i64>(0)?,"createdAt":r.get::<_,i64>(1)?,"transport":r.get::<_,String>(2)?,"operation":r.get::<_,String>(3)?,"targetId":r.get::<_,Option<String>>(4)?,"outcome":r.get::<_,String>(5)?,"detail":r.get::<_,String>(6)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
            Ok(json!({"retentionDays":60,"activity":rows}))
        })())
    }
}

#[prompt_router]
impl OdoMcp {
    #[prompt(name = "daily_planning")]
    async fn daily_planning(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Review my Odo planner tasks and today's journal. Build a realistic daily plan with priorities, time blocks, and a small finish line. Focus: {}",args.focus.unwrap_or_else(||"balanced progress".into())))])
    }
    #[prompt(name = "inbox_triage")]
    async fn inbox_triage(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Review Odo Inbox notes and unscheduled planner tasks. Propose folders, task conversions, archival, and next actions without permanently deleting anything. Focus: {}",args.focus.unwrap_or_else(||"clarity".into())))])
    }
    #[prompt(name = "weekly_review")]
    async fn weekly_review(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Use this week's Odo notes, completed and open tasks, and journal entries to run a weekly review: wins, unfinished work, lessons, and next-week priorities. Focus: {}",args.focus.unwrap_or_else(||"sustainable momentum".into())))])
    }
    #[prompt(name = "meeting_note_processing")]
    async fn meeting_note_processing(
        &self,
        Parameters(args): Parameters<PromptArgs>,
    ) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![PromptMessage::new_text(Role::User,format!("Process the relevant Odo meeting note into decisions, owners, deadlines, follow-ups, and planner tasks. Preserve the original note. Focus: {}",args.focus.unwrap_or_else(||"clear accountability".into())))])
    }
    #[prompt(name = "overdue_task_review")]
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
                Resource::new("odo://workspace", "Odo workspace summary"),
                Resource::new("odo://folders", "Odo folder tree"),
                Resource::new("odo://notes", "Odo notes index"),
                Resource::new("odo://tasks", "Odo planner tasks"),
                Resource::new("odo://journal", "Odo journal index"),
                Resource::new("odo://activity", "Odo MCP activity (60 days)"),
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
                ResourceTemplate::new("odo://notes/{id}", "Odo note"),
                ResourceTemplate::new("odo://journal/{date}", "Odo journal day"),
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
            Ok(value) => Ok(ReadResourceResult::new(vec![ResourceContents::text(
                value.to_string(),
                uri,
            )])),
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
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token()),
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
