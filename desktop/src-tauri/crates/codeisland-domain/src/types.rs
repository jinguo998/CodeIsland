use chrono::{DateTime, Utc};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Every agent CodeIsland can show. The tag is the stable wire/source name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum AgentKind {
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "gemini")]
    Gemini,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "opencode")]
    OpenCode,
    #[serde(rename = "qoder")]
    Qoder,
    #[serde(rename = "droid")]
    Factory,
    #[serde(rename = "codebuddy")]
    CodeBuddy,
    #[serde(rename = "copilot")]
    Copilot,
    #[serde(rename = "grok")]
    Grok,
    #[serde(rename = "kimi")]
    Kimi,
    #[serde(rename = "cline")]
    Cline,
    #[serde(rename = "pi")]
    Pi,
    #[serde(rename = "qwen")]
    Qwen,
    #[serde(rename = "hermes")]
    Hermes,
    #[serde(rename = "antigravity")]
    Antigravity,
    #[serde(rename = "kiro")]
    Kiro,
    #[serde(rename = "stepfun")]
    StepFun,
    #[serde(rename = "workbuddy")]
    WorkBuddy,
    #[serde(rename = "openclaw")]
    OpenClaw,
    #[serde(rename = "zcode")]
    ZCode,
    #[serde(rename = "minimax")]
    MiniMax,
    #[serde(rename = "deepseek")]
    DeepSeek,
    #[serde(rename = "aiwork")]
    AiWork,
    #[serde(rename = "mimo")]
    MiMo,
    #[serde(rename = "trae")]
    Trae,
    #[serde(rename = "traecn")]
    TraeCn,
    #[serde(rename = "traecli")]
    TraeCli,
    #[serde(rename = "codybuddycn")]
    CodyBuddyCn,
    #[serde(rename = "qodercn")]
    QoderCn,
    #[serde(rename = "qoderwork")]
    QoderWork,
    #[serde(rename = "unknown")]
    Unknown,
}

impl AgentKind {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
            Self::OpenCode => "opencode",
            Self::Qoder => "qoder",
            Self::Factory => "droid",
            Self::CodeBuddy => "codebuddy",
            Self::Copilot => "copilot",
            Self::Grok => "grok",
            Self::Kimi => "kimi",
            Self::Cline => "cline",
            Self::Pi => "pi",
            Self::Qwen => "qwen",
            Self::Hermes => "hermes",
            Self::Antigravity => "antigravity",
            Self::Kiro => "kiro",
            Self::StepFun => "stepfun",
            Self::WorkBuddy => "workbuddy",
            Self::OpenClaw => "openclaw",
            Self::ZCode => "zcode",
            Self::MiniMax => "minimax",
            Self::DeepSeek => "deepseek",
            Self::AiWork => "aiwork",
            Self::MiMo => "mimo",
            Self::Trae => "trae",
            Self::TraeCn => "traecn",
            Self::TraeCli => "traecli",
            Self::CodyBuddyCn => "codybuddycn",
            Self::QoderCn => "qodercn",
            Self::QoderWork => "qoderwork",
            Self::Unknown => "unknown",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini CLI",
            Self::Cursor => "Cursor",
            Self::OpenCode => "OpenCode",
            Self::Qoder => "Qoder",
            Self::Factory => "Factory Droid",
            Self::CodeBuddy => "CodeBuddy",
            Self::Copilot => "GitHub Copilot CLI",
            Self::Grok => "Grok CLI",
            Self::Kimi => "Kimi Code CLI",
            Self::Cline => "Cline",
            Self::Pi => "Pi",
            Self::Qwen => "Qwen Code",
            Self::Hermes => "Hermes",
            Self::Antigravity => "AntiGravity",
            Self::Kiro => "Kiro",
            Self::StepFun => "StepFun",
            Self::WorkBuddy => "WorkBuddy",
            Self::OpenClaw => "OpenClaw",
            Self::ZCode => "ZCode",
            Self::MiniMax => "MiniMax Code CLI",
            Self::DeepSeek => "DeepSeek Harness",
            Self::AiWork => "AiWork",
            Self::MiMo => "MiMo",
            Self::Trae => "Trae",
            Self::TraeCn => "Trae CN",
            Self::TraeCli => "Trae CLI",
            Self::CodyBuddyCn => "CodyBuddyCN",
            Self::QoderCn => "Qoder CN",
            Self::QoderWork => "QoderWork",
            Self::Unknown => "Unknown",
        }
    }

    pub fn from_tag(tag: &str) -> Self {
        match tag.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" => Self::Claude,
            "codex" => Self::Codex,
            "gemini" | "gemini-cli" => Self::Gemini,
            "cursor" | "cursor-cli" => Self::Cursor,
            "opencode" => Self::OpenCode,
            "qoder" => Self::Qoder,
            "droid" | "factory" => Self::Factory,
            "codebuddy" => Self::CodeBuddy,
            "copilot" => Self::Copilot,
            "grok" => Self::Grok,
            "kimi" => Self::Kimi,
            "cline" => Self::Cline,
            "pi" | "omp" | "oh-my-pi" => Self::Pi,
            "qwen" => Self::Qwen,
            "hermes" => Self::Hermes,
            "antigravity" | "google-antigravity" => Self::Antigravity,
            "kiro" => Self::Kiro,
            "stepfun" => Self::StepFun,
            "workbuddy" => Self::WorkBuddy,
            "openclaw" => Self::OpenClaw,
            "zcode" => Self::ZCode,
            "minimax" => Self::MiniMax,
            "deepseek" | "dsh" => Self::DeepSeek,
            "aiwork" | "aiwork-cli" => Self::AiWork,
            "mimo" => Self::MiMo,
            "trae" => Self::Trae,
            "traecn" => Self::TraeCn,
            "traecli" => Self::TraeCli,
            "codybuddycn" => Self::CodyBuddyCn,
            "qodercn" => Self::QoderCn,
            "qoderwork" => Self::QoderWork,
            _ => Self::Unknown,
        }
    }
}

impl Default for AgentKind {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventType {
    SessionStart,
    SessionEnd,
    UserPrompt,
    ToolStart,
    ToolEnd,
    PermissionRequest,
    Question,
    Message,
    Error,
    Heartbeat,
    TurnEnd,
    TurnFailed,
    Interrupt,
    Notification,
    SubagentStart,
    SubagentEnd,
}

impl AgentEventType {
    pub fn is_blocking(self) -> bool {
        matches!(self, Self::PermissionRequest | Self::Question)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct AgentEvent {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Subagent identity when the event came from a spawned agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent: AgentKind,
    #[serde(rename = "type")]
    pub event_type: AgentEventType,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Public assistant text identified by the local Codex rollout parser.
    #[serde(skip)]
    #[ts(skip)]
    pub codex_transcript_output: Option<String>,
    #[serde(skip)]
    #[ts(skip)]
    pub transcript_metadata: Option<crate::session_metadata::TranscriptMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wezterm_pane: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_event_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// Why the turn stopped, when the agent reports it (`user`, `interrupted`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// True when the agent hook is blocked waiting for a decision.
    #[serde(default)]
    pub waits: bool,
}

impl AgentEvent {
    pub fn new(session_id: impl Into<String>, agent: AgentKind, event_type: AgentEventType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            seq: None,
            session_id: session_id.into(),
            parent_session_id: None,
            agent_id: None,
            agent,
            event_type,
            timestamp: Utc::now(),
            cwd: None,
            tool: None,
            text: None,
            codex_transcript_output: None,
            transcript_metadata: None,
            title: None,
            model: None,
            reasoning: None,
            pid: None,
            terminal_kind: None,
            wezterm_pane: None,
            raw_event_name: None,
            transcript_path: None,
            branch: None,
            payload: None,
            stop_reason: None,
            waits: false,
        }
    }

    /// A turn the user stopped is a cancellation, not a completion. Agents
    /// report the stop reason on the turn-ending event instead of an Interrupt.
    pub fn is_interrupted_turn(&self) -> bool {
        matches!(
            self.event_type,
            AgentEventType::TurnEnd | AgentEventType::TurnFailed
        ) && self.stop_reason.as_deref().is_some_and(|reason| {
            matches!(
                reason.to_ascii_lowercase().as_str(),
                "user" | "interrupted" | "aborted" | "cancelled"
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Idle,
    Starting,
    Running,
    WaitingPermission,
    WaitingAnswer,
    Completed,
    Failed,
    Cancelled,
}

impl SessionStatus {
    pub fn needs_you(self) -> bool {
        matches!(self, Self::WaitingPermission | Self::WaitingAnswer)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct AgentTask {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct HistoryEntry {
    pub at: DateTime<Utc>,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Verified Linux process and terminal identity for a local WSL session.
/// Linux PIDs are scoped to the distribution and are never Win32 PIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslProcess {
    pub distro: String,
    pub pid: u32,
    pub start_ticks: u64,
    pub tty: Option<String>,
    pub windows_terminal_session: Option<String>,
    pub wezterm_pane: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SessionSnapshot {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Subagent identity when this card is a spawned agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub agent: AgentKind,
    pub status: SessionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    /// Public assistant text seen live during the current Codex turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_codex_output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tool_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    /// Claude Desktop's Code-tab identifier for exact session navigation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_desktop_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    // Computed by the shared Swift model formatter when the UI snapshot is built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recap: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub recap_created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub last_prompt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip)]
    #[ts(skip)]
    pub discovered_pid_started_at: Option<DateTime<Utc>>,
    /// Rebuilt from WSL process evidence while the app is running.
    #[serde(skip)]
    #[ts(skip)]
    pub wsl_process: Option<WslProcess>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wezterm_pane: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(default)]
    pub interrupted: bool,
    #[serde(default)]
    pub last_turn_failed: bool,
    #[serde(default)]
    pub is_executing_tool: bool,
    #[serde(default)]
    pub child_session_ids: Vec<String>,
    #[serde(default)]
    pub always_allowed_tools: Vec<String>,
    #[serde(skip)]
    #[ts(skip)]
    pub always_allowed_command_prefixes: Vec<Vec<String>>,
    #[serde(default)]
    pub tasks: Vec<AgentTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub tasks_updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

impl SessionSnapshot {
    pub fn new(event: &AgentEvent) -> Self {
        Self {
            session_id: event.session_id.clone(),
            parent_session_id: event.parent_session_id.clone(),
            agent_id: event.agent_id.clone(),
            agent: event.agent,
            // A fresh card rests until an event proves it is working; a
            // SessionStart is not itself work.
            status: SessionStatus::Idle,
            title: event.title.clone(),
            cwd: event.cwd.clone(),
            project: project_name(event.cwd.as_deref()),
            started_at: event.timestamp,
            last_activity: event.timestamp,
            last_message: None,
            live_codex_output: None,
            last_prompt: None,
            active_tool: None,
            active_tool_detail: None,
            provider_session_id: None,
            claude_desktop_session_id: None,
            tool_use_id: None,
            model: event.model.clone(),
            model_label: None,
            reasoning: event.reasoning.clone(),
            recap: None,
            recap_created_at: None,
            last_prompt_at: None,
            error: None,
            pid: event.pid,
            discovered_pid_started_at: None,
            wsl_process: None,
            terminal_kind: event.terminal_kind.clone(),
            wezterm_pane: event.wezterm_pane,
            transcript_path: event.transcript_path.clone(),
            git_branch: event.branch.clone(),
            interrupted: false,
            last_turn_failed: false,
            is_executing_tool: false,
            child_session_ids: Vec::new(),
            always_allowed_tools: Vec::new(),
            always_allowed_command_prefixes: Vec::new(),
            tasks: Vec::new(),
            tasks_updated_at: None,
            history: Vec::new(),
        }
    }

    pub fn is_child(&self) -> bool {
        self.parent_session_id.is_some()
    }
}

pub fn project_name(cwd: Option<&str>) -> Option<String> {
    let cwd = cwd?.trim();
    if cwd.is_empty() {
        return None;
    }
    let trimmed = cwd.trim_end_matches(['\\', '/']);
    trimmed
        .rsplit(['\\', '/'])
        .next()
        .filter(|value| !value.is_empty() && !value.ends_with(':'))
        .map(ToOwned::to_owned)
        .or_else(|| Some(trimmed.to_owned()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    Permission,
    Question,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct QuestionItem {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub option_descriptions: Vec<String>,
    #[serde(default)]
    pub is_secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct PendingRequest {
    pub event_id: String,
    pub session_id: String,
    #[serde(default)]
    pub agent: AgentKind,
    pub kind: RequestKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub questions: Vec<QuestionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub hidden: bool,
    /// Request correlation belongs to the store; it is not a UI payload.
    #[serde(skip)]
    #[ts(skip)]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<serde_json::Value>,
    #[serde(default)]
    pub always_scope: crate::ApprovalAlwaysScope,
    #[serde(skip)]
    #[ts(skip)]
    pub approval_config: Option<std::path::PathBuf>,
    /// The agent keeps a native question visible while the hook waits.
    #[serde(skip)]
    #[ts(skip)]
    pub native_ask_racing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum UserAction {
    Approve,
    Deny,
    AlwaysAllow,
    Answer,
    Skip,
    Hide,
}

impl PendingRequest {
    pub fn accepts(&self, action: UserAction) -> bool {
        match action {
            UserAction::Hide => true,
            UserAction::Approve | UserAction::Deny | UserAction::AlwaysAllow => {
                self.kind == RequestKind::Permission
            }
            UserAction::Answer | UserAction::Skip => self.kind == RequestKind::Question,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct Decision {
    pub event_id: String,
    pub session_id: String,
    pub agent: AgentKind,
    pub action: UserAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct StoreSnapshot {
    pub sessions: Vec<SessionSnapshot>,
    pub pending: Vec<PendingRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_session_id: Option<String>,
    pub total_sessions: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub sessions: IndexMap<String, SessionSnapshot>,
    #[serde(default)]
    pub manual_order: Vec<String>,
    #[serde(default)]
    pub saved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ApplyOutcome {
    pub applied: bool,
    pub session_id: String,
    pub is_new_session: bool,
    pub ended_turn: bool,
    pub turn_failed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<PendingRequest>,
}
