use std::{
    path::PathBuf,
    sync::Arc,
};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::mcp;

#[derive(Clone)]
pub struct ApiState {
    pub db_path: PathBuf,
    pub notifier: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl ApiState {
    pub fn open(&self) -> Result<Connection, OdoError> {
        mcp::open(&self.db_path).map_err(|error| OdoError::Internal(error))
    }

    pub fn notify(&self) {
        if let Some(notifier) = &self.notifier {
            notifier();
        }
    }
}

#[derive(Serialize)]
pub struct ErrorBody {
    error: String,
}

#[derive(Debug)]
pub enum OdoError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    Internal(String),
}

impl OdoError {
    fn status(&self) -> StatusCode {
        match self {
            OdoError::NotFound(_) => StatusCode::NOT_FOUND,
            OdoError::BadRequest(_) => StatusCode::BAD_REQUEST,
            OdoError::Conflict(_) => StatusCode::CONFLICT,
            OdoError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn message(&self) -> String {
        match self {
            OdoError::NotFound(message)
            | OdoError::BadRequest(message)
            | OdoError::Conflict(message)
            | OdoError::Internal(message) => message.clone(),
        }
    }
}

impl IntoResponse for OdoError {
    fn into_response(self) -> Response {
        let status = self.status();
        let body = Json(ErrorBody {
            error: self.message(),
        });
        (status, body).into_response()
    }
}

impl From<String> for OdoError {
    fn from(error: String) -> Self {
        OdoError::Internal(error)
    }
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ListNotesQuery {
    folder_id: Option<String>,
    status: Option<String>,
    query: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    include_content: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateNoteBody {
    folder_id: Option<String>,
    title: String,
    content: Option<String>,
    pinned: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateNoteBody {
    expected_revision: i64,
    title: Option<String>,
    content: Option<String>,
    folder_id: Option<String>,
    status: Option<String>,
    pinned: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ListTasksQuery {
    completed: Option<bool>,
    category_id: Option<String>,
    scheduled_from: Option<String>,
    scheduled_to: Option<String>,
    query: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTaskBody {
    text: String,
    category_id: Option<String>,
    priority: Option<String>,
    effort: Option<i64>,
    color: Option<String>,
    scheduled_start: Option<String>,
    duration_minutes: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateTaskBody {
    text: Option<String>,
    completed: Option<bool>,
    category_id: Option<String>,
    priority: Option<String>,
    effort: Option<i64>,
    color: Option<String>,
    scheduled_start: Option<String>,
    clear_schedule: Option<bool>,
    duration_minutes: Option<i64>,
}

fn paginate(rows: &mut Vec<Value>, limit: u32) -> (bool, u32) {
    let has_more = rows.len() > limit as usize;
    if has_more {
        rows.truncate(limit as usize);
    }
    (has_more, limit)
}

fn now_iso_utc() -> Result<DateTime<Utc>, OdoError> {
    let stamp = mcp::now_iso();
    DateTime::parse_from_rfc3339(&stamp)
        .map(|dt: DateTime<FixedOffset>| dt.with_timezone(&Utc))
        .map_err(|error| OdoError::Internal(format!("Could not parse current timestamp: {error}")))
}

fn ics_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
        .replace('\r', "")
}

fn format_ics_timestamp(dt: &DateTime<Utc>) -> String {
    dt.format("%Y%m%dT%H%M%SZ").to_string()
}

async fn workspace_summary(State(state): State<ApiState>) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    let count = |table: &str| -> Result<i64, OdoError> {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .map_err(|error| OdoError::Internal(error.to_string()))
    };
    Ok(Json(json!({
        "folders": count("folders")?,
        "notes": count("notes")?,
        "tasks": count("todos")?,
        "categories": count("todo_categories")?,
        "journalEntries": count("journal_entries")?,
        "projects": count("projects")?,
        "milestones": count("milestones")?,
    })))
}

async fn folders_list(State(state): State<ApiState>) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    let mut statement = connection
        .prepare(
            "SELECT f.id,f.name,f.parent_id,f.icon,f.position,COUNT(n.id) FROM folders f LEFT JOIN notes n ON n.folder_id=f.id GROUP BY f.id ORDER BY f.position",
        )
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    let rows: Vec<Value> = statement
        .query_map([], |row| {
            let id = row.get::<_, String>(0)?;
            let stored_icon = row.get::<_, Option<String>>(3)?;
            let icon = if id == "inbox" {
                stored_icon.unwrap_or_else(|| "ph-tray".into())
            } else {
                mcp::normalize_folder_icon(stored_icon.as_deref()).into()
            };
            Ok(json!({
                "id": id,
                "name": row.get::<_, String>(1)?,
                "parentId": row.get::<_, Option<String>>(2)?,
                "icon": icon,
                "position": row.get::<_, i64>(4)?,
                "noteCount": row.get::<_, i64>(5)?,
            }))
        })
        .map_err(|error| OdoError::Internal(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    Ok(Json(json!({"folders": rows, "allowedIcons": mcp::ALLOWED_FOLDER_ICONS.as_slice()})))
}

async fn notes_list(
    State(state): State<ApiState>,
    Query(query): Query<ListNotesQuery>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);
    if limit == 0 {
        return Ok(Json(mcp::empty_paginated("notes", offset)));
    }

    let mut sql = "SELECT id,folder_id,title,content,updated,status,pinned,revision FROM notes WHERE 1=1".to_string();
    let mut values: Vec<String> = Vec::new();
    if let Some(folder_id) = &query.folder_id {
        sql.push_str(" AND folder_id=?");
        values.push(folder_id.clone());
    }
    if let Some(status) = &query.status {
        sql.push_str(" AND status=?");
        values.push(status.clone());
    }
    if let Some(q) = query.query.filter(|q| !q.trim().is_empty()) {
        sql.push_str(" AND (title LIKE ? OR content LIKE ?)");
        let pattern = format!("%{q}%");
        values.push(pattern.clone());
        values.push(pattern);
    }
    sql.push_str(" ORDER BY updated DESC LIMIT ? OFFSET ?");
    values.push((limit + 1).to_string());
    values.push(offset.to_string());

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    let mut rows: Vec<Value> = statement
        .query_map(rusqlite::params_from_iter(values.iter()), |row| {
            let content: String = row.get(3)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "folderId": row.get::<_, String>(1)?,
                "title": row.get::<_, String>(2)?,
                "content": if query.include_content.unwrap_or(false) { Value::String(content.clone()) } else { Value::Null },
                "excerpt": content.chars().take(240).collect::<String>(),
                "updated": row.get::<_, String>(4)?,
                "status": row.get::<_, String>(5)?,
                "pinned": row.get::<_, bool>(6)?,
                "revision": row.get::<_, i64>(7)?,
            }))
        })
        .map_err(|error| OdoError::Internal(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    let (has_more, _) = paginate(&mut rows, limit);

    Ok(Json(json!({
        "notes": rows,
        "limit": limit,
        "offset": offset,
        "hasMore": has_more,
        "nextOffset": if has_more { Some(offset + limit) } else { None },
    })))
}

async fn notes_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    let trimmed = id.trim();
    match mcp::OdoMcp::note_json(&connection, trimmed) {
        Ok(note) => Ok(Json(note)),
        Err(_) if !trimmed.is_empty() => {
            mcp::OdoMcp::note_json_by_title(&connection, trimmed)
                .map(Json)
                .map_err(|error| OdoError::NotFound(error))
        }
        Err(error) => Err(OdoError::NotFound(error)),
    }
}

async fn notes_create(
    State(state): State<ApiState>,
    Json(body): Json<CreateNoteBody>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    mcp::validate_note_title(&body.title)?;
    let id = format!("note-{}", Uuid::new_v4());
    let folder_id = body.folder_id.unwrap_or_else(|| "inbox".into());
    let updated = mcp::now_iso();
    connection
        .execute(
            "INSERT INTO notes(id,folder_id,title,content,updated,status,pinned,position,revision)
             VALUES(?1,?2,?3,?4,?5,'active',?6,(SELECT COALESCE(MAX(position),-1)+1 FROM notes),0)",
            params![
                id,
                folder_id,
                body.title,
                body.content.unwrap_or_default(),
                updated,
                body.pinned.unwrap_or(false)
            ],
        )
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    mcp::bump_change(&connection).map_err(OdoError::Internal)?;
    mcp::audit(&connection, "http", "notes.create", Some(&id), "success", "content redacted");
    state.notify();
    mcp::OdoMcp::note_json(&connection, &id)
        .map(Json)
        .map_err(OdoError::Internal)
}

async fn notes_update(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateNoteBody>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    let current = mcp::OdoMcp::note_json(&connection, &id)
        .map_err(|_| OdoError::NotFound(format!("Note '{id}' was not found")))?;
    let current_revision = current["revision"].as_i64().unwrap_or_default();
    if current_revision != body.expected_revision {
        return Err(OdoError::Conflict(format!(
            "Revision conflict: expected {}, but the note is now revision {}.",
            body.expected_revision, current_revision
        )));
    }

    let title = body
        .title
        .unwrap_or_else(|| current["title"].as_str().unwrap_or_default().into());
    mcp::validate_note_title(&title)?;
    let content = body
        .content
        .unwrap_or_else(|| current["content"].as_str().unwrap_or_default().into());
    let folder_id = body
        .folder_id
        .unwrap_or_else(|| current["folderId"].as_str().unwrap_or("inbox").into());
    let status = body
        .status
        .unwrap_or_else(|| current["status"].as_str().unwrap_or("active").into());
    let pinned = body
        .pinned
        .unwrap_or(current["pinned"].as_bool().unwrap_or(false));
    if !matches!(status.as_str(), "active" | "archived" | "trash") {
        return Err(OdoError::BadRequest("status must be active, archived, or trash".into()));
    }
    let updated = mcp::now_iso();

    let count = connection
        .execute(
            "UPDATE notes SET folder_id=?2,title=?3,content=?4,updated=?5,status=?6,pinned=?7,revision=revision+1
             WHERE id=?1 AND revision=?8",
            params![
                id,
                folder_id,
                title,
                content,
                updated,
                status,
                pinned,
                body.expected_revision
            ],
        )
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    if count == 0 {
        return Err(OdoError::Conflict(
            "The note changed before the update could be saved".into(),
        ));
    }
    mcp::bump_change(&connection).map_err(OdoError::Internal)?;
    mcp::audit(&connection, "http", "notes.update", Some(&id), "success", "content redacted");
    state.notify();
    mcp::OdoMcp::note_json(&connection, &id)
        .map(Json)
        .map_err(OdoError::Internal)
}

async fn notes_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    if !matches!(id.as_str(), "welcome" | "inbox") {
        connection
            .execute(
                "UPDATE notes SET status='trash',updated=?2,revision=revision+1 WHERE id=?1 AND status='active'",
                params![id, mcp::now_iso()],
            )
            .map_err(|error| OdoError::Internal(error.to_string()))?;
    }
    mcp::bump_change(&connection).map_err(OdoError::Internal)?;
    mcp::audit(&connection, "http", "notes.delete", Some(&id), "success", "moved to trash");
    state.notify();
    Ok(Json(json!({"id": id, "status": "trash"})))
}

async fn tasks_list(
    State(state): State<ApiState>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);
    if limit == 0 {
        return Ok(Json(mcp::empty_paginated("tasks", offset)));
    }

    let mut sql = "SELECT id,text,completed,created,updated,category_id,priority,effort,color,scheduled_start,duration_minutes FROM todos WHERE 1=1".to_string();
    let mut values: Vec<String> = Vec::new();
    if let Some(completed) = query.completed {
        sql.push_str(" AND completed=?");
        values.push(if completed { "1".into() } else { "0".into() });
    }
    if let Some(category_id) = &query.category_id {
        sql.push_str(" AND category_id=?");
        values.push(category_id.clone());
    }
    if let Some(from) = &query.scheduled_from {
        sql.push_str(" AND scheduled_start IS NOT NULL AND scheduled_start>=?");
        values.push(from.clone());
    }
    if let Some(to) = &query.scheduled_to {
        sql.push_str(" AND scheduled_start IS NOT NULL AND scheduled_start<=?");
        values.push(to.clone());
    }
    if let Some(q) = query.query.filter(|q| !q.trim().is_empty()) {
        sql.push_str(" AND text LIKE ?");
        values.push(format!("%{q}%"));
    }
    sql.push_str(" ORDER BY position LIMIT ? OFFSET ?");
    values.push((limit + 1).to_string());
    values.push(offset.to_string());

    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    let mut rows: Vec<Value> = statement
        .query_map(rusqlite::params_from_iter(values.iter()), |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "text": row.get::<_, String>(1)?,
                "completed": row.get::<_, bool>(2)?,
                "created": row.get::<_, String>(3)?,
                "updated": row.get::<_, String>(4)?,
                "categoryId": row.get::<_, String>(5)?,
                "priority": row.get::<_, String>(6)?,
                "effort": row.get::<_, i64>(7)?,
                "color": row.get::<_, String>(8)?,
                "scheduledStart": row.get::<_, Option<String>>(9)?,
                "durationMinutes": row.get::<_, i64>(10)?,
            }))
        })
        .map_err(|error| OdoError::Internal(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    let (has_more, _) = paginate(&mut rows, limit);

    Ok(Json(json!({
        "tasks": rows,
        "limit": limit,
        "offset": offset,
        "hasMore": has_more,
        "nextOffset": if has_more { Some(offset + limit) } else { None },
    })))
}

async fn tasks_get(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    mcp::OdoMcp::task_json(&connection, &id)
        .map(Json)
        .map_err(|error| OdoError::NotFound(error))
}

async fn tasks_create(
    State(state): State<ApiState>,
    Json(body): Json<CreateTaskBody>,
) -> Result<Json<Value>, OdoError> {
    mcp::validate_task_text(&body.text)?;
    let priority = mcp::validate_task_priority(body.priority)?;
    let color = mcp::validate_task_color(body.color)?;
    let scheduled_start = mcp::validate_scheduled_start(body.scheduled_start)?;
    let duration_minutes = mcp::normalize_task_duration(body.duration_minutes)?;

    let connection = state.open()?;
    let id = format!("todo-{}", Uuid::new_v4());
    let stamp = mcp::now_iso();
    connection
        .execute(
            "INSERT INTO todos(id,text,completed,created,updated,position,category_id,priority,effort,color,scheduled_start,duration_minutes)
             VALUES(?1,?2,0,?3,?3,(SELECT COALESCE(MAX(position),-1)+1 FROM todos),?4,?5,?6,?7,?8,?9)",
            params![
                id,
                body.text,
                stamp,
                body.category_id.unwrap_or_else(|| "inbox".into()),
                priority,
                body.effort.unwrap_or(2).clamp(1, 5),
                color,
                scheduled_start,
                duration_minutes,
            ],
        )
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    mcp::bump_change(&connection).map_err(OdoError::Internal)?;
    mcp::audit(&connection, "http", "tasks.create", Some(&id), "success", "");
    state.notify();
    mcp::OdoMcp::task_json(&connection, &id)
        .map(Json)
        .map_err(OdoError::Internal)
}

async fn tasks_update(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    let current: (String, bool, String, String, i64, String, Option<String>, i64) = connection
        .query_row(
            "SELECT text,completed,category_id,priority,effort,color,scheduled_start,duration_minutes FROM todos WHERE id=?1",
            [&id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(|error| OdoError::Internal(error.to_string()))?
        .ok_or_else(|| OdoError::NotFound(format!("Task '{id}' was not found")))?;

    let text = body.text.unwrap_or(current.0);
    mcp::validate_task_text(&text)?;
    let scheduled = if body.clear_schedule.unwrap_or(false) {
        None
    } else {
        body.scheduled_start.or(current.6)
    };
    let duration_minutes = body.duration_minutes.unwrap_or(current.7).max(30);
    let priority = mcp::validate_task_priority(body.priority.clone().or(Some(current.3)))?;
    let color = mcp::validate_task_color(body.color.or(Some(current.5)))?;

    connection
        .execute(
            "UPDATE todos SET text=?2,completed=?3,updated=?4,category_id=?5,priority=?6,effort=?7,color=?8,scheduled_start=?9,duration_minutes=?10 WHERE id=?1",
            params![
                id,
                text,
                body.completed.unwrap_or(current.1),
                mcp::now_iso(),
                body.category_id.unwrap_or(current.2),
                priority,
                body.effort.unwrap_or(current.4).clamp(1, 5),
                color,
                scheduled,
                duration_minutes,
            ],
        )
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    mcp::bump_change(&connection).map_err(OdoError::Internal)?;
    mcp::audit(&connection, "http", "tasks.update", Some(&id), "success", "");
    state.notify();
    mcp::OdoMcp::task_json(&connection, &id)
        .map(Json)
        .map_err(OdoError::Internal)
}

async fn tasks_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, OdoError> {
    let connection = state.open()?;
    if connection
        .execute("DELETE FROM todos WHERE id=?1", [&id])
        .map_err(|error| OdoError::Internal(error.to_string()))?
        == 0
    {
        return Err(OdoError::NotFound(format!("Task '{id}' was not found")));
    }
    mcp::bump_change(&connection).map_err(OdoError::Internal)?;
    mcp::audit(&connection, "http", "tasks.delete", Some(&id), "success", "");
    state.notify();
    Ok(Json(json!({"deleted": true, "id": id})))
}

fn build_ics(connection: &Connection) -> Result<String, OdoError> {
    let mut statement = connection
        .prepare(
            "SELECT id,text,scheduled_start,duration_minutes,content FROM todos
             WHERE scheduled_start IS NOT NULL AND completed=0
             ORDER BY scheduled_start",
        )
        .map_err(|error| OdoError::Internal(error.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| OdoError::Internal(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| OdoError::Internal(error.to_string()))?;

    let mut ics = String::new();
    ics.push_str("BEGIN:VCALENDAR\r\n");
    ics.push_str("VERSION:2.0\r\n");
    ics.push_str("PRODID:-//Odo Tasks//EN\r\n");
    ics.push_str("CALSCALE:GREGORIAN\r\n");
    ics.push_str("METHOD:PUBLISH\r\n");
    ics.push_str("X-WR-CALNAME:Odo Tasks\r\n");

    let now = now_iso_utc()?;
    let dtstamp = format_ics_timestamp(&now);

    for (id, text, scheduled, duration, content) in rows {
        let start = DateTime::parse_from_rfc3339(&scheduled).map_err(|error| {
            OdoError::Internal(format!("Could not parse scheduled start for {id}: {error}"))
        })?;
        let start_utc: DateTime<Utc> = start.with_timezone(&Utc);
        let end_utc: DateTime<Utc> = (start + Duration::minutes(duration))
            .with_timezone(&Utc);

        ics.push_str("BEGIN:VEVENT\r\n");
        ics.push_str(&format!("UID:odo-task-{}@odo\r\n", id));
        ics.push_str(&format!("DTSTAMP:{}\r\n", dtstamp));
        ics.push_str(&format!("DTSTART:{}\r\n", format_ics_timestamp(&start_utc)));
        ics.push_str(&format!("DTEND:{}\r\n", format_ics_timestamp(&end_utc)));
        ics.push_str(&format!("SUMMARY:{}\r\n", ics_escape(&text)));
        if !content.is_empty() {
            ics.push_str(&format!("DESCRIPTION:{}\r\n", ics_escape(&content)));
        } else {
            ics.push_str("DESCRIPTION:\r\n");
        }
        ics.push_str("END:VEVENT\r\n");
    }

    ics.push_str("END:VCALENDAR\r\n");
    Ok(ics)
}

async fn calendar_ics(State(state): State<ApiState>) -> Result<Response, OdoError> {
    let connection = state.open()?;
    let body = build_ics(&connection)?;
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/calendar; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok((headers, body).into_response())
}

fn openapi_spec() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Odo Tasks API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "A local REST API for notes, planner tasks, and the calendar feed. Requires the same bearer token as the MCP server when authentication is enabled."
        },
        "servers": [{"url": "/api/v1"}],
        "paths": {
            "/workspace": {
                "get": {
                    "summary": "Workspace summary",
                    "operationId": "workspaceSummary",
                    "responses": {
                        "200": {"description": "Counts for folders, notes, tasks, categories, journal entries, projects, and milestones"}
                    }
                }
            },
            "/folders": {
                "get": {
                    "summary": "List folders",
                    "operationId": "foldersList",
                    "responses": {"200": {"description": "Nested folder tree"}}
                }
            },
            "/notes": {
                "get": {
                    "summary": "List notes",
                    "operationId": "notesList",
                    "parameters": [
                        {"name": "folderId", "in": "query", "schema": {"type": "string"}},
                        {"name": "status", "in": "query", "schema": {"type": "string"}},
                        {"name": "query", "in": "query", "schema": {"type": "string"}},
                        {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                        {"name": "offset", "in": "query", "schema": {"type": "integer"}},
                        {"name": "includeContent", "in": "query", "schema": {"type": "boolean"}}
                    ],
                    "responses": {"200": {"description": "Paginated notes"}}
                },
                "post": {
                    "summary": "Create a note",
                    "operationId": "notesCreate",
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/CreateNote"}}}
                    },
                    "responses": {"200": {"description": "Created note"}}
                }
            },
            "/notes/{id}": {
                "get": {
                    "summary": "Get a note",
                    "operationId": "notesGet",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "Note"}, "404": {"description": "Not found"}}
                },
                "patch": {
                    "summary": "Update a note",
                    "operationId": "notesUpdate",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpdateNote"}}}
                    },
                    "responses": {"200": {"description": "Updated note"}}
                },
                "delete": {
                    "summary": "Move a note to trash",
                    "operationId": "notesDelete",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "Trashed"}}
                }
            },
            "/tasks": {
                "get": {
                    "summary": "List tasks",
                    "operationId": "tasksList",
                    "parameters": [
                        {"name": "completed", "in": "query", "schema": {"type": "boolean"}},
                        {"name": "categoryId", "in": "query", "schema": {"type": "string"}},
                        {"name": "scheduledFrom", "in": "query", "schema": {"type": "string"}},
                        {"name": "scheduledTo", "in": "query", "schema": {"type": "string"}},
                        {"name": "query", "in": "query", "schema": {"type": "string"}},
                        {"name": "limit", "in": "query", "schema": {"type": "integer"}},
                        {"name": "offset", "in": "query", "schema": {"type": "integer"}}
                    ],
                    "responses": {"200": {"description": "Paginated tasks"}}
                },
                "post": {
                    "summary": "Create a task",
                    "operationId": "tasksCreate",
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/CreateTask"}}}
                    },
                    "responses": {"200": {"description": "Created task"}}
                }
            },
            "/tasks/{id}": {
                "get": {
                    "summary": "Get a task",
                    "operationId": "tasksGet",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "Task"}}
                },
                "patch": {
                    "summary": "Update a task",
                    "operationId": "tasksUpdate",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "requestBody": {
                        "required": true,
                        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpdateTask"}}}
                    },
                    "responses": {"200": {"description": "Updated task"}}
                },
                "delete": {
                    "summary": "Delete a task",
                    "operationId": "tasksDelete",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "Deleted"}}
                }
            },
            "/calendar.ics": {
                "get": {
                    "summary": "iCalendar feed of scheduled tasks",
                    "operationId": "calendarIcs",
                    "parameters": [{"name": "token", "in": "query", "schema": {"type": "string"}, "description": "MCP bearer token when authentication is enabled"}],
                    "responses": {
                        "200": {"description": "iCalendar data", "content": {"text/calendar": {}}}
                    }
                }
            },
            "/openapi.json": {
                "get": {
                    "summary": "OpenAPI specification",
                    "operationId": "openapiJson",
                    "responses": {"200": {"description": "OpenAPI JSON"}}
                }
            }
        },
        "components": {
            "schemas": {
                "CreateNote": {
                    "type": "object",
                    "properties": {
                        "folderId": {"type": "string"},
                        "title": {"type": "string", "minLength": 1, "maxLength": 1000},
                        "content": {"type": "string"},
                        "pinned": {"type": "boolean"}
                    },
                    "required": ["title"]
                },
                "UpdateNote": {
                    "type": "object",
                    "properties": {
                        "expectedRevision": {"type": "integer"},
                        "title": {"type": "string", "minLength": 1, "maxLength": 1000},
                        "content": {"type": "string"},
                        "folderId": {"type": "string"},
                        "status": {"type": "string", "enum": ["active", "archived", "trash"]},
                        "pinned": {"type": "boolean"}
                    },
                    "required": ["expectedRevision"]
                },
                "CreateTask": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "minLength": 1},
                        "categoryId": {"type": "string"},
                        "priority": {"type": "string", "enum": ["low", "medium", "high"]},
                        "effort": {"type": "integer", "minimum": 1, "maximum": 5},
                        "color": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"},
                        "scheduledStart": {"type": "string", "format": "date-time"},
                        "durationMinutes": {"type": "integer", "minimum": 1, "maximum": 1440}
                    },
                    "required": ["text"]
                },
                "UpdateTask": {
                    "type": "object",
                    "properties": {
                        "text": {"type": "string", "minLength": 1},
                        "completed": {"type": "boolean"},
                        "categoryId": {"type": "string"},
                        "priority": {"type": "string", "enum": ["low", "medium", "high"]},
                        "effort": {"type": "integer", "minimum": 1, "maximum": 5},
                        "color": {"type": "string", "pattern": "^#[0-9A-Fa-f]{6}$"},
                        "scheduledStart": {"type": "string", "format": "date-time"},
                        "clearSchedule": {"type": "boolean"},
                        "durationMinutes": {"type": "integer", "minimum": 1, "maximum": 1440}
                    }
                }
            }
        }
    })
}

async fn openapi_json() -> Json<Value> {
    Json(openapi_spec())
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/workspace", get(workspace_summary))
        .route("/folders", get(folders_list))
        .route("/notes", get(notes_list).post(notes_create))
        .route("/notes/{id}", get(notes_get).patch(notes_update).delete(notes_delete))
        .route("/tasks", get(tasks_list).post(tasks_create))
        .route("/tasks/{id}", get(tasks_get).patch(tasks_update).delete(tasks_delete))
        .route("/calendar.ics", get(calendar_ics))
        .route("/openapi.json", get(openapi_json))
        .with_state(state)
}

