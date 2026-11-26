# CodeIsland Desktop — 内部接口契约

所有 crate 必须遵守本文件的公开 API。domain 类型见 `src-tauri/crates/codeisland-domain/src/types.rs`。

## codeisland-ipc

```rust
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello { v: u32, id: String, #[serde(default)] source: Option<String> },
    Event {
        v: u32, id: String, source: String,
        event: AgentEvent,
        #[serde(default)] waits: bool,
        #[serde(default)] timeout_ms: Option<u64>,
    },
    Heartbeat { v: u32, id: String },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerMessage {
    Ack { v: u32, id: String, ack_for: String },
    Decision {
        v: u32, id: String, ack_for: String,
        action: UserAction,
        #[serde(default)] text: Option<String>,
        stdout: String,
        exit_code: i32,
    },
    Error { v: u32, id: String, error: String },
}

/// `\\.\pipe\codeisland-<sid hash>` on Windows; `$XDG_RUNTIME_DIR/codeisland-<uid>.sock` on Unix.
pub fn endpoint_name() -> String;

pub type Handler = std::sync::Arc<
    dyn Fn(ClientMessage) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<ServerMessage>> + Send>
    > + Send + Sync,
>;

/// Accepts connections forever. One NDJSON request and one NDJSON response per connection.
pub async fn serve(handler: Handler) -> std::io::Result<()>;

pub mod client {
    use super::*;
    /// Blocking one-shot client used by codeisland-hook. Retries ERROR_PIPE_BUSY.
    pub fn send(
        message: &ClientMessage,
        connect_timeout: std::time::Duration,
        read_timeout: std::time::Duration,
    ) -> std::io::Result<ServerMessage>;
}
```

Named pipe ACL: current user SID only (plus SYSTEM). The pipe is message/byte stream; NDJSON framing.

## codeisland-agents

```rust
pub struct AgentDefinition {
    pub agent: AgentKind,
    pub display_name: String,
    pub executables: Vec<String>,
    pub config_paths: Vec<String>,
    pub format: HookFormat,
    pub events: Vec<HookEventDef>,
    pub source: String,
    pub response: ResponseFormat,
    pub icon: String,
    pub docs_url: Option<String>,
}
pub struct HookEventDef { pub name: String, pub timeout_ms: u64, pub blocking: bool }
pub enum HookFormat { ClaudeJson, EventMapJson, CursorJson, OpenCodePlugin, GenericJson, TomlBlock, YamlBlock, PluginJs, Manual }
pub enum ResponseFormat { Claude, Codex, Gemini, Cursor, Notification, Plain }

pub fn definitions() -> &'static [AgentDefinition];
pub fn definition(agent: AgentKind) -> Option<&'static AgentDefinition>;
pub fn definition_for_source(source: &str) -> Option<&'static AgentDefinition>;

pub fn normalize(source: &str, event_name: &str, raw: &str, cwd_hint: Option<&str>) -> Result<AgentEvent, AgentError>;

/// Pre-rendered stdout the hook prints. `fallback` is printed when there is no decision.
pub fn render_response(agent: AgentKind, event: &AgentEvent, decision: Option<&Decision>, fallback: &str) -> (String, i32);

pub struct ToolStatus {
    pub agent: AgentKind,
    pub display_name: String,
    pub installed: bool,
    pub hook_installed: bool,
    pub healthy: bool,
    pub config_path: Option<String>,
    pub command: Option<String>,
    pub problem: Option<String>,
}
pub fn detect(agent: AgentKind) -> ToolStatus;
pub fn detect_all() -> Vec<ToolStatus>;
pub fn install(agent: AgentKind) -> Result<ToolStatus, AgentError>;
pub fn uninstall(agent: AgentKind) -> Result<ToolStatus, AgentError>;
pub fn repair(agent: AgentKind) -> Result<ToolStatus, AgentError>;
pub fn hook_command(agent: AgentKind) -> String;
```

`hook_command` resolves the `codeisland-hook.exe` path: env `CODEISLAND_HOOK` override, else beside the running executable, else `codeisland-hook` on PATH.

Normalization rules (minimum):
- Claude: `hook_event_name`, `session_id`, `cwd`, `tool_name`, `tool_input`, `message`, `permission_request`; `AskUserQuestion` → Question.
- Codex: `session_id`, `cwd`, `tool_name`, `type`; permission → PermissionRequest.
- Gemini: `session_id`/`sessionId`, `cwd`, `tool_name`, `type`.
- Cursor: `session_id`/`conversation_id`, `cwd`, `tool_name`, `hook_event_name`.
- Unknown source: best-effort mapping from `event_name`; never fail the hook (return a generic event or error only for unparseable JSON).

Response schemas:
- Claude permission approve: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}`; deny `"deny"`; always adds `updatedPermissions` (`addRules`, session).
- Claude question: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":{...original tool_input, "answers":{"answer":[text]}}}}}`; Notification question: `{"hookSpecificOutput":{"hookEventName":"Notification","answer":text}}`.
- Codex: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"|"always"|"deny"}}}`.
- Gemini/Cursor/other: JSON `{"decision":"allow"|"deny"}` or `{}` fallback; never crash.

## codeisland-runtime

```rust
pub struct UiState {
    pub store: StoreSnapshot,
    pub settings: AppSettings,
    pub version: String,
}

#[derive(Clone, Debug)]
pub enum RuntimeUpdate {
    State(StoreSnapshot),
    Attention { session_id: String, decision: AttentionDecision },
    Settings(AppSettings),
    Toast { kind: String, message: String },
}

pub struct Runtime { /* Arc-friendly */ }

impl Runtime {
    pub fn new(settings: AppSettings) -> std::sync::Arc<Self>;
    pub fn state(&self) -> UiState;
    pub fn settings(&self) -> AppSettings;
    pub fn update_settings(&self, settings: AppSettings) -> AppSettings;

    pub fn handle_event(self: &Arc<Self>, event: AgentEvent) -> ApplyOutcome;
    pub async fn handle_blocking(self: &Arc<Self>, event: AgentEvent, timeout: std::time::Duration) -> Option<Decision>;

    pub fn resolve(&self, event_id: &str, action: UserAction, text: Option<String>) -> Option<Decision>;
    pub fn resolve_current(&self, action: UserAction, text: Option<String>) -> Option<Decision>;
    pub fn hide(&self, event_id: &str) -> bool;
    pub fn unhide(&self, event_id: &str) -> bool;
    pub fn remove_session(&self, session_id: &str) -> bool;
    pub fn move_session(&self, session_id: &str, before_id: &str) -> bool;
    pub fn dismiss_completed(&self) -> usize;

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<RuntimeUpdate>;

    /// Starts the IPC server, cleanup task and Codex tailer. Returns a shutdown handle.
    pub fn start(self: &Arc<Self>) -> RuntimeHandle;
}

pub struct RuntimeHandle { /* Drop stops tasks */ }
```

`handle_event` applies the event, computes attention via `codeisland_domain::attention`, emits `State` + `Attention`, and persists (debounced). `handle_blocking` registers a oneshot waiter, applies the event, awaits `resolve`, renders stdout with `codeisland_agents::render_response`, and returns the decision.

## Tauri 应用（desktop/src-tauri/src）

Events emitted to the webview:
- `island://state` → `UiState`
- `island://attention` → `{ sessionId, decision }`
- `island://toast` → `{ kind, message }`

Commands (`invoke`):
- `get_state() -> UiState`
- `take_launch_sound() -> bool` (main island only, once per manual launch)
- `resolve_request(eventId, action, text?) -> Option<Decision>`
- `resolve_current(action, text?) -> Option<Decision>`
- `hide_request(eventId) -> bool`
- `unhide_request(eventId) -> bool`
- `remove_session(sessionId) -> bool`
- `move_session(sessionId, beforeId) -> bool`
- `dismiss_completed() -> usize`
- `get_settings() -> AppSettings`
- `update_settings(settings) -> AppSettings`
- `hook_status() -> Vec<ToolStatus>`
- `install_hooks(agent?) -> Vec<ToolStatus>`
- `repair_hooks(agent?) -> Vec<ToolStatus>`
- `uninstall_hooks(agent?) -> Vec<ToolStatus>`
- `export_diagnostics(path) -> String`
- `open_workspace(sessionId) -> bool`
- `toggle_collapsed() -> bool`
- `open_settings()`
- `quit_app()`

TypeScript 侧使用 `src/lib/bindings/` 中 ts-rs 生成的类型；命令参数为 camelCase。
