use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::mcp;

const AI_CONFIG_KEY: &str = "ai_config_v1";
const KEYRING_SERVICE: &str = "Odo Tasks";
const KEYRING_ACCOUNT: &str = "openai-api-key";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub web_search_enabled: bool,
    #[serde(default)]
    pub auto_create_tasks: bool,
    #[serde(default = "default_duration_minutes")]
    pub default_duration_minutes: i64,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_provider(),
            model: default_model(),
            web_search_enabled: true,
            auto_create_tasks: false,
            default_duration_minutes: default_duration_minutes(),
        }
    }
}

fn default_provider() -> String {
    "openai".into()
}

fn default_model() -> String {
    "gpt-4o".into()
}

fn default_duration_minutes() -> i64 {
    60
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub config: AiConfig,
    pub masked_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiMessageRequest {
    pub text: String,
    #[serde(default)]
    pub history: Vec<AiHistoryMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiHistoryMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiMessageResponse {
    pub response: String,
    #[serde(default)]
    pub created_tasks: Vec<Value>,
    #[serde(default)]
    pub used_search: bool,
}

pub fn load_ai_config(path: &PathBuf) -> Result<AiConfig, String> {
    let connection = mcp::open(path)?;
    let value: Option<String> = connection
        .query_row(
            "SELECT value FROM app_state WHERE key=?1",
            [AI_CONFIG_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("Could not load AI settings: {error}"))?;
    match value {
        Some(value) => serde_json::from_str(&value)
            .map_err(|error| format!("The saved AI settings are invalid: {error}")),
        None => Ok(AiConfig::default()),
    }
}

pub fn save_ai_config(path: &PathBuf, config: &AiConfig) -> Result<(), String> {
    let connection = mcp::open(path)?;
    let value = serde_json::to_string(config)
        .map_err(|error| format!("Could not encode AI settings: {error}"))?;
    connection
        .execute(
            "INSERT INTO app_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![AI_CONFIG_KEY, value],
        )
        .map_err(|error| format!("Could not save AI settings: {error}"))?;
    Ok(())
}

fn open_keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
        .map_err(|error| format!("Could not open the OS credential store: {error}"))
}

pub fn load_api_key() -> Result<Option<String>, String> {
    match open_keyring_entry() {
        Ok(entry) => match entry.get_password() {
            Ok(key) if !key.trim().is_empty() => Ok(Some(key)),
            Ok(_) => Ok(None),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("Could not read the OpenAI API key: {error}")),
        },
        Err(error) => Err(error),
    }
}

pub fn save_api_key(key: &str) -> Result<(), String> {
    let entry = open_keyring_entry()?;
    entry
        .set_password(key)
        .map_err(|error| format!("Could not save the OpenAI API key: {error}"))
}

pub fn delete_api_key() -> Result<(), String> {
    match open_keyring_entry() {
        Ok(entry) => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("Could not remove the OpenAI API key: {error}")),
        },
        Err(_) => Ok(()),
    }
}

pub fn masked_api_key() -> String {
    match load_api_key() {
        Ok(Some(key)) if key.len() > 8 => {
            format!("{}••••{}", &key[..4], &key[key.len() - 4..])
        }
        Ok(Some(_)) => "••••••".into(),
        Ok(None) => "Not set".into(),
        Err(error) => format!("Error: {error}"),
    }
}

pub fn ai_settings(path: &PathBuf) -> Result<AiSettings, String> {
    let config = load_ai_config(path)?;
    let masked_key = masked_api_key();
    Ok(AiSettings {
        config,
        masked_key,
    })
}

pub fn update_ai_settings(
    path: &PathBuf,
    config: AiConfig,
    new_key: Option<String>,
) -> Result<(), String> {
    if let Some(key) = new_key {
        if key.trim().is_empty() {
            delete_api_key()?;
        } else {
            save_api_key(&key)?;
        }
    }
    save_ai_config(path, &config)
}

// TODO(user): customize this system prompt. It shapes how the AI talks to you,
// what events it looks for, and when it asks for confirmation before adding tasks.
pub fn system_prompt() -> String {
    let now = mcp::now_iso();
    format!(
        "You are Odo's built-in assistant. The current time is {now}.\n\n\
        Your job is to help the user manage their Odo workspace, especially to find real-world events and add them to the planner schedule.\n\n\
        Tools available:\n\
        - web_search: use this to look up concerts, conferences, meetups, webinars, sports, and other real-world events when the user asks.\n\
        - odo_list_tasks: list existing tasks to avoid duplicates.\n\
        - odo_search_workspace: search notes, tasks, and journal entries.\n\
        - odo_get_current_time: get the current date and time if you are unsure.\n\
        - odo_create_scheduled_task: create a new scheduled task. Only call this when the user explicitly asks you to add something to their schedule, or when you are confident from context.\n\n\
        When adding events:\n\
        1. Use web_search to verify the event name, date, and time.\n\
        2. Convert the event's local date/time to a timezone-qualified ISO-8601 timestamp (e.g. 2026-09-01T19:00:00-04:00) for the scheduledStart argument.\n\
        3. Use durationMinutes between 30 and 180 for most events unless you have specific information.\n\
        4. Use the event name as the task text.\n\
        5. If the event date or time is unclear, ask the user before creating the task.\n\n\
        Keep your final responses concise, friendly, and useful. Cite the source of any event you looked up."
    )
}

fn default_database_path() -> Result<PathBuf, String> {
    mcp::default_database_path()
}

fn open_db() -> Result<Connection, String> {
    mcp::open(&default_database_path()?)
}

fn odo_tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "name": "odo_list_tasks",
            "description": "List the user's planner tasks. Use this to avoid duplicating an event or to see what is already scheduled.",
            "parameters": {
                "type": "object",
                "properties": {
                    "completed": { "type": "boolean", "description": "If true, return completed tasks; if false, return open tasks; if omitted, return all." },
                    "query": { "type": "string" },
                    "scheduledFrom": { "type": "string", "format": "date-time" },
                    "scheduledTo": { "type": "string", "format": "date-time" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100 }
                }
            }
        }),
        json!({
            "type": "function",
            "name": "odo_create_scheduled_task",
            "description": "Create a new scheduled task for an event. Only use when the user wants to add the event to their schedule.",
            "parameters": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Short task text, usually the event name." },
                    "scheduledStart": { "type": "string", "format": "date-time", "description": "Timezone-qualified ISO-8601 start time." },
                    "durationMinutes": { "type": "integer", "minimum": 1, "maximum": 1440, "description": "How long the event lasts." },
                    "priority": { "type": "string", "enum": ["low", "medium", "high"] },
                    "color": { "type": "string", "pattern": "^#[0-9A-Fa-f]{6}$" },
                    "categoryId": { "type": "string" }
                },
                "required": ["text", "scheduledStart"]
            }
        }),
        json!({
            "type": "function",
            "name": "odo_search_workspace",
            "description": "Search the user's notes, tasks, and journal for any text.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50 }
                },
                "required": ["query"]
            }
        }),
        json!({
            "type": "function",
            "name": "odo_get_current_time",
            "description": "Get the current date and time in RFC3339 format.",
            "parameters": { "type": "object", "properties": {} }
        }),
    ]
}

fn odo_list_tasks(args: &Value) -> Result<Value, String> {
    let connection = open_db()?;
    let completed = args.get("completed").and_then(|v| v.as_bool());
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let scheduled_from = args
        .get("scheduledFrom")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let scheduled_to = args
        .get("scheduledTo")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(20)
        .clamp(1, 100) as usize;

    let mut sql = "SELECT id,text,completed,category_id,priority,effort,color,scheduled_start,duration_minutes FROM todos WHERE 1=1".to_string();
    let mut values: Vec<String> = Vec::new();
    if let Some(completed) = completed {
        sql.push_str(" AND completed=?");
        values.push(if completed { "1".into() } else { "0".into() });
    }
    if let Some(from) = scheduled_from {
        sql.push_str(" AND scheduled_start IS NOT NULL AND scheduled_start>=?");
        values.push(from);
    }
    if let Some(to) = scheduled_to {
        sql.push_str(" AND scheduled_start IS NOT NULL AND scheduled_start<=?");
        values.push(to);
    }
    if let Some(query) = query.filter(|q| !q.trim().is_empty()) {
        sql.push_str(" AND text LIKE ?");
        values.push(format!("%{query}%"));
    }
    sql.push_str(" ORDER BY position LIMIT ?");
    values.push(limit.to_string());

    let mut statement = connection.prepare(&sql).map_err(|e| e.to_string())?;
    let rows: Vec<Value> = statement
        .query_map(rusqlite::params_from_iter(values.iter()), |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "text": row.get::<_, String>(1)?,
                "completed": row.get::<_, bool>(2)?,
                "categoryId": row.get::<_, String>(3)?,
                "priority": row.get::<_, String>(4)?,
                "effort": row.get::<_, i64>(5)?,
                "color": row.get::<_, String>(6)?,
                "scheduledStart": row.get::<_, Option<String>>(7)?,
                "durationMinutes": row.get::<_, i64>(8)?,
            }))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(json!({ "tasks": rows }))
}

fn odo_create_scheduled_task(args: &Value, _config: &AiConfig) -> Result<Value, String> {
    let text = args
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Task text is required".to_string())?;
    mcp::validate_task_text(text)?;
    let scheduled_start = args
        .get("scheduledStart")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let scheduled_start = mcp::validate_scheduled_start(scheduled_start)?;
    let duration = mcp::normalize_task_duration(args.get("durationMinutes").and_then(|v| v.as_i64()))?;
    let priority = mcp::validate_task_priority(
        args.get("priority")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    )?;
    let color = mcp::validate_task_color(
        args.get("color")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    )?;
    let category_id = args
        .get("categoryId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "inbox".into());

    let connection = open_db()?;
    let id = format!("todo-{}", Uuid::new_v4());
    let stamp = mcp::now_iso();
    connection
        .execute(
            "INSERT INTO todos(id,text,completed,created,updated,position,category_id,priority,effort,color,scheduled_start,duration_minutes)
             VALUES(?1,?2,0,?3,?3,(SELECT COALESCE(MAX(position),-1)+1 FROM todos),?4,?5,?6,?7,?8,?9)",
            params![
                id,
                text,
                stamp,
                category_id,
                priority,
                2i64,
                color,
                scheduled_start,
                duration,
            ],
        )
        .map_err(|e| e.to_string())?;
    mcp::bump_change(&connection)?;
    mcp::audit(&connection, "ai", "tasks.create", Some(&id), "success", "");
    mcp::OdoMcp::task_json(&connection, &id)
}

fn odo_search_workspace(args: &Value) -> Result<Value, String> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if query.is_empty() {
        return Ok(json!({ "results": [] }));
    }
    let limit = args
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(20)
        .clamp(1, 50) as i64;
    let pattern = format!("%{query}%");
    let connection = open_db()?;
    let mut statement = connection.prepare(
        "SELECT kind,id,label,excerpt FROM (
            SELECT 'note' AS kind, id, title AS label, substr(content,1,160) AS excerpt, updated AS sort_key
            FROM notes WHERE title LIKE ?1 OR content LIKE ?1
            UNION ALL
            SELECT 'task', id, text, NULL, updated FROM todos WHERE text LIKE ?1
            UNION ALL
            SELECT 'journal', id, date_key, substr(content,1,160), updated FROM journal_entries WHERE content LIKE ?1 OR date_key LIKE ?1
        ) ORDER BY sort_key DESC LIMIT ?2 OFFSET 0",
    ).map_err(|e| e.to_string())?;
    let rows: Vec<Value> = statement
        .query_map(params![pattern, limit], |row| {
            Ok(json!({
                "kind": row.get::<_, String>(0)?,
                "id": row.get::<_, String>(1)?,
                "label": row.get::<_, String>(2)?,
                "excerpt": row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            }))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(json!({ "results": rows }))
}

fn odo_get_current_time() -> Result<Value, String> {
    Ok(json!({ "now": mcp::now_iso() }))
}

fn execute_odo_function(
    name: &str,
    arguments: &str,
    config: &AiConfig,
) -> Result<Value, String> {
    let args: Value = serde_json::from_str(arguments)
        .map_err(|e| format!("Could not parse function arguments: {e}"))?;
    match name {
        "odo_list_tasks" => odo_list_tasks(&args),
        "odo_create_scheduled_task" => {
            if !config.auto_create_tasks {
                return Ok(json!({"error": "auto-creation of tasks is disabled. Ask the user to enable it in AI settings or confirm before adding."}));
            }
            odo_create_scheduled_task(&args, config)
        }
        "odo_search_workspace" => odo_search_workspace(&args),
        "odo_get_current_time" => odo_get_current_time(),
        _ => Err(format!("Unknown Odo function: {name}")),
    }
}

fn build_input(request: &AiMessageRequest) -> Vec<Value> {
    let mut input: Vec<Value> = request
        .history
        .iter()
        .map(|message| {
            json!({
                "role": message.role.clone(),
                "content": message.content.clone(),
            })
        })
        .collect();
    input.push(json!({
        "role": "user",
        "content": request.text.clone(),
    }));
    input
}

fn build_tools(config: &AiConfig) -> Vec<Value> {
    let mut tools = odo_tool_definitions();
    if config.web_search_enabled {
        tools.insert(0, json!({"type": "web_search"}));
    }
    tools
}

async fn call_openai_responses(
    client: &reqwest::Client,
    api_key: &str,
    config: &AiConfig,
    input: &[Value],
) -> Result<Value, String> {
    let body = json!({
        "model": config.model,
        "instructions": system_prompt(),
        "input": input,
        "tools": build_tools(config),
        "tool_choice": "auto",
        "store": false,
    });

    let response = client
        .post(OPENAI_RESPONSES_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("Could not reach OpenAI: {error}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .await
        .map_err(|error| format!("Could not read OpenAI response: {error}"))?;
    if !status.is_success() {
        return Err(format!("OpenAI returned {status}: {body_text}"));
    }

    serde_json::from_str(&body_text)
        .map_err(|error| format!("OpenAI response was not valid JSON: {error}"))
}

pub async fn send_message(
    request: AiMessageRequest,
    config: &AiConfig,
    api_key: &str,
) -> Result<AiMessageResponse, String> {
    let client = reqwest::Client::new();
    let mut input = build_input(&request);
    let mut created_tasks: Vec<Value> = Vec::new();
    let mut used_search = false;

    for _turn in 0..5 {
        let response = call_openai_responses(&client, api_key, config, &input).await?;

        if let Some(output) = response.get("output").and_then(|v| v.as_array()) {
            // Append all output items to the input for the next turn.
            input.extend(output.iter().cloned());

            // Track whether any web search happened.
            if output.iter().any(|item| {
                item.get("type")
                    .and_then(|v| v.as_str())
                    .map_or(false, |t| t == "web_search_call")
            }) {
                used_search = true;
            }

            // Execute any Odo function calls and append their outputs.
            let mut function_outputs: Vec<Value> = Vec::new();
            for item in output {
                if item.get("type").and_then(|v| v.as_str()) != Some("function_call") {
                    continue;
                }
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let call_id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let arguments = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");

                if name.starts_with("odo_") {
                    let result = execute_odo_function(name, arguments, config)?;
                    if name == "odo_create_scheduled_task" {
                        if let Some(task) = result.as_object() {
                            created_tasks.push(Value::Object(task.clone()));
                        }
                    }
                    function_outputs.push(json!({
                        "type": "function_call_output",
                        "call_id": call_id,
                        "output": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
                    }));
                }
            }

            if function_outputs.is_empty() {
                // No Odo calls remain; the assistant is done.
                let response_text = response
                    .get("output_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                return Ok(AiMessageResponse {
                    response: response_text,
                    created_tasks,
                    used_search,
                });
            }

            input.extend(function_outputs);
        } else {
            let response_text = response
                .get("output_text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return Ok(AiMessageResponse {
                response: response_text,
                created_tasks,
                used_search,
            });
        }
    }

    Err("The assistant made too many tool calls. Please try a simpler question.".into())
}
