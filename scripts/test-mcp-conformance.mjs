import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import readline from "node:readline";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const binary = join(root, "src-tauri", "target", "debug", "odo-tasks");
const temporaryDirectory = mkdtempSync(join(tmpdir(), "odo-mcp-conformance-"));
const database = join(temporaryDirectory, "workspace.sqlite3");
const config = JSON.stringify({
  enabled: true,
  host: "127.0.0.1",
  port: 8765,
  authEnabled: false,
  token: "conformance-test-token",
  permanentDeleteEnabled: false,
  startAtLogin: false,
});

const schema = `
PRAGMA journal_mode=WAL;
CREATE TABLE folders (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, parent_id TEXT, is_open INTEGER NOT NULL,
  icon TEXT, position INTEGER NOT NULL
);
CREATE TABLE notes (
  id TEXT PRIMARY KEY, folder_id TEXT NOT NULL, title TEXT NOT NULL, content TEXT NOT NULL,
  updated TEXT NOT NULL, status TEXT NOT NULL, pinned INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL, revision INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE todos (
  id TEXT PRIMARY KEY, text TEXT NOT NULL, completed INTEGER NOT NULL DEFAULT 0,
  created TEXT NOT NULL, updated TEXT NOT NULL, position INTEGER NOT NULL,
  category_id TEXT NOT NULL DEFAULT 'inbox', priority TEXT NOT NULL DEFAULT 'medium',
  effort INTEGER NOT NULL DEFAULT 2, color TEXT NOT NULL DEFAULT '', scheduled_start TEXT,
  duration_minutes INTEGER NOT NULL DEFAULT 30
);
CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE todo_categories (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL, icon TEXT NOT NULL DEFAULT '',
  position INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE journal_entries (
  id TEXT PRIMARY KEY, date_key TEXT NOT NULL UNIQUE, content TEXT NOT NULL DEFAULT '',
  created TEXT NOT NULL, updated TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0
);
INSERT INTO folders VALUES ('inbox','Inbox',NULL,1,'ph-tray',0);
INSERT INTO todo_categories VALUES ('inbox','Inbox','#7b8e7c','ph-tray',0);
INSERT INTO app_state(key,value) VALUES ('mcp_config_v1', '${config.replaceAll("'", "''")}');
`;

const sqlite = spawnSync("sqlite3", [database], { input: schema, encoding: "utf8" });
assert.equal(sqlite.status, 0, sqlite.stderr);

const child = spawn(binary, ["--mcp-stdio"], {
  cwd: root,
  env: { ...process.env, ODO_DATABASE_PATH: database },
  stdio: ["pipe", "pipe", "pipe"],
});
const lines = readline.createInterface({ input: child.stdout });
const pending = new Map();
let nextId = 1;
let standardError = "";
child.stderr.on("data", (chunk) => {
  standardError += chunk.toString();
});
lines.on("line", (line) => {
  if (!line.trim()) return;
  const message = JSON.parse(line);
  if (message.id === undefined) return;
  const request = pending.get(message.id);
  if (!request) return;
  pending.delete(message.id);
  clearTimeout(request.timer);
  if (message.error) request.reject(new Error(message.error.message));
  else request.resolve(message.result);
});

function rpc(method, params = {}) {
  const id = nextId++;
  return new Promise((resolvePromise, reject) => {
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`Timed out waiting for ${method}. stderr: ${standardError}`));
    }, 5_000);
    pending.set(id, { resolve: resolvePromise, reject, timer });
    child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
}

function notify(method, params = {}) {
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method, params })}\n`);
}

function decodeToolResult(result) {
  const text = result?.content?.find((item) => item.type === "text")?.text ?? "";
  let value = text;
  try {
    value = JSON.parse(text);
  } catch {
    // Error tool responses intentionally use plain text.
  }
  return { isError: result?.isError === true, text, value };
}

async function tool(name, args = {}) {
  return decodeToolResult(await rpc("tools/call", { name, arguments: args }));
}

async function toolOk(name, args = {}) {
  const result = await tool(name, args);
  assert.equal(result.isError, false, `${name} failed: ${result.text}`);
  return result.value;
}

async function toolFails(name, args, expected) {
  try {
    const result = await tool(name, args);
    assert.equal(result.isError, true, `${name} unexpectedly succeeded: ${result.text}`);
    assert.match(result.text, expected);
  } catch (error) {
    assert.match(String(error), expected);
  }
}

try {
  const initialize = await rpc("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "odo-conformance", version: "1.0.0" },
  });
  assert.equal(initialize.serverInfo.name, "odo-mcp");
  notify("notifications/initialized");

  const listedTools = await rpc("tools/list");
  const tools = new Map(listedTools.tools.map((entry) => [entry.name, entry]));
  const requiredTools = [
    "activity_list", "batch_apply", "folders_create", "folders_update", "journal_delete",
    "journal_get", "notes_append", "notes_create", "notes_get", "notes_list",
    "notes_prepend", "notes_update", "search", "tasks_create", "tasks_list",
    "tasks_update", "workspace_backup",
  ];
  for (const name of requiredTools) assert(tools.has(name), `Missing tool ${name}`);
  for (const name of ["notes_list", "tasks_list", "search", "activity_list"]) {
    assert(tools.get(name).inputSchema.properties.offset, `${name} must advertise offset`);
  }
  assert(tools.get("tasks_list").inputSchema.properties.query, "tasks_list must advertise query");
  assert.match(tools.get("notes_list").description, /Content is excluded by default/i);
  assert.match(tools.get("notes_append").description, /after the existing content and before the new text/i);
  assert.match(tools.get("notes_prepend").description, /after the new text and before the existing content/i);
  assert.match(tools.get("activity_list").description, /Content fields are redacted in the audit log for privacy/);
  assert.match(tools.get("batch_apply").description, /requires an operation field/);
  assert.match(tools.get("batch_apply").description, /expectedRevision/);
  assert.deepEqual(Object.keys(tools.get("journal_get").inputSchema.properties), ["dateKey"]);
  assert.deepEqual(Object.keys(tools.get("journal_delete").inputSchema.properties), ["dateKey"]);
  assert.equal(tools.get("notes_create").inputSchema.properties.title.minLength, 1);
  assert.equal(tools.get("notes_create").inputSchema.properties.title.maxLength, 1000);

  const prompts = (await rpc("prompts/list")).prompts;
  assert.equal(prompts.length, 5);
  for (const prompt of prompts) assert(prompt.description?.trim(), `${prompt.name} lacks a description`);

  const resources = (await rpc("resources/list")).resources;
  assert.equal(resources.length, 6);
  for (const resource of resources) {
    assert(resource.description?.trim(), `${resource.uri} lacks a description`);
    assert.equal(resource.mimeType, "text/markdown");
  }
  assert.match(resources.find((resource) => resource.uri === "odo://notes").description, /odo:\/\/notes\/\{id\}/);
  assert.match(resources.find((resource) => resource.uri === "odo://journal").description, /odo:\/\/journal\/\{date\}/);
  const templates = (await rpc("resources/templates/list")).resourceTemplates;
  assert.equal(templates.length, 2);
  for (const template of templates) {
    assert(template.description?.trim());
    assert.equal(template.mimeType, "text/markdown");
  }
  const workspaceResource = await rpc("resources/read", { uri: "odo://workspace" });
  assert.equal(workspaceResource.contents[0].mimeType, "text/markdown");

  await toolFails("notes_create", { title: "" }, /Note title cannot be empty|validation/i);
  await toolFails("notes_create", { title: "x".repeat(1001) }, /1000|1,000|validation/i);
  const note = await toolOk("notes_create", { title: "UniqueTitle12345XYZ", content: "base" });
  assert.equal(note.title, "UniqueTitle12345XYZ");
  assert.equal((await toolOk("notes_get", { title: "uniquetitle12345xyz" })).id, note.id);
  assert.equal((await toolOk("notes_get", { id: "UniqueTitle12345XYZ" })).id, note.id);
  const listedWithoutContent = await toolOk("notes_list", { limit: 1 });
  assert.equal(listedWithoutContent.notes[0].content, null);
  const listedWithContent = await toolOk("notes_list", { limit: 1, includeContent: true });
  assert.equal(typeof listedWithContent.notes[0].content, "string");
  const zeroNotes = await toolOk("notes_list", { limit: 0, offset: 41 });
  assert.deepEqual(zeroNotes, { notes: [], limit: 0, offset: 41, hasMore: false, nextOffset: null });
  const appended = await toolOk("notes_append", {
    id: note.id, expectedRevision: note.revision, text: "after", separator: "---\n",
  });
  assert.equal(appended.content, "base---\nafter");
  const prepended = await toolOk("notes_prepend", {
    id: note.id, expectedRevision: appended.revision, text: "before", separator: "\n--\n",
  });
  assert.equal(prepended.content, "before\n--\nbase---\nafter");
  await toolFails("notes_update", {
    id: note.id, expectedRevision: prepended.revision, title: " ",
  }, /Note title cannot be empty|validation/i);

  await toolOk("notes_create", { title: "Second paged note", content: "page marker" });
  const firstPage = await toolOk("notes_list", { limit: 1, offset: 0 });
  assert.equal(firstPage.hasMore, true);
  assert.equal(firstPage.nextOffset, 1);
  const secondPage = await toolOk("notes_list", { limit: 1, offset: firstPage.nextOffset });
  assert.notEqual(firstPage.notes[0].id, secondPage.notes[0].id);
  await toolFails("search", { query: "" }, /Query must not be empty/);
  const zeroSearch = await toolOk("search", { query: "note", limit: 0, offset: 9 });
  assert.deepEqual(zeroSearch, { query: "note", results: [], limit: 0, offset: 9, hasMore: false, nextOffset: null });

  const rootFolder = await toolOk("folders_create", { name: "Root fallback", parentId: "folder-missing", icon: "ph-book" });
  assert.equal(rootFolder.parentId, null);
  assert.equal(rootFolder.name, "Root fallback");
  await toolFails("folders_create", { name: "Bad icon", icon: "ph-trash" }, /not allowed|validation/i);
  const childFolder = await toolOk("folders_create", { name: "Child", parentId: rootFolder.id });
  const updatedFolder = await toolOk("folders_update", { id: childFolder.id, name: "Child renamed" });
  assert.equal(updatedFolder.name, "Child renamed");
  assert.equal(updatedFolder.parentId, rootFolder.id);
  await toolFails("folders_update", { id: rootFolder.id, parentId: childFolder.id }, /circular/);
  assert.equal(typeof (await toolOk("workspace_summary")).folders, "number");

  const task = await toolOk("tasks_create", { text: "Protocol searchable task" });
  assert.equal(task.text, "Protocol searchable task");
  assert.equal(task.completed, false);
  const updatedTask = await toolOk("tasks_update", { id: task.id, text: "Protocol updated task", completed: true });
  assert.equal(updatedTask.text, "Protocol updated task");
  assert.equal(updatedTask.completed, true);
  const filteredTasks = await toolOk("tasks_list", { query: "UPDATED", limit: 10 });
  assert.deepEqual(filteredTasks.tasks.map((item) => item.id), [task.id]);
  const zeroTasks = await toolOk("tasks_list", { limit: 0, offset: 13 });
  assert.deepEqual(zeroTasks, { tasks: [], limit: 0, offset: 13, hasMore: false, nextOffset: null });

  const category = await toolOk("categories_create", { name: "Protocol category", color: "#123456", icon: "ph-star" });
  assert.equal(category.name, "Protocol category");
  assert.equal(category.color, "#123456");
  const backup = await toolOk("workspace_backup");
  assert.equal(backup.created, true);
  assert.equal(typeof backup.backupId, "string");
  assert.equal("path" in backup, false);
  assert.equal(backup.backupId.includes("/"), false);
  const activity = await toolOk("activity_list", { limit: 100 });
  assert(activity.activity.some((entry) => entry.detail === "content redacted"));
  const zeroActivity = await toolOk("activity_list", { limit: 0, offset: 7 });
  assert.deepEqual(zeroActivity, { activity: [], limit: 0, offset: 7, hasMore: false, nextOffset: null, retentionDays: 60 });

  console.log(`MCP conformance passed: ${listedTools.tools.length} tools, ${prompts.length} prompts, ${resources.length} resources, ${templates.length} templates.`);
} finally {
  child.stdin.end();
  child.kill("SIGTERM");
  await new Promise((resolvePromise) => child.once("close", resolvePromise));
  rmSync(temporaryDirectory, { recursive: true, force: true });
}
