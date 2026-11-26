//! ESP32 "Buddy" companion protocol.
//!
//! Ported from `Sources/CodeIslandCore/ESP32Protocol.swift`. The byte layouts
//! are firmware contracts: change them only together with the Buddy firmware.
//!
//! BLE service / characteristics:
//! - service `0000beef-0000-1000-8000-00805f9b34fb`
//! - write (host -> Buddy) `0000beef-0001-1000-8000-00805f9b34fb`
//! - notify (Buddy -> host) `0000beef-0002-1000-8000-00805f9b34fb`
//!
//! The pure encoders/decoders are always available; [`BuddyManager`] is a
//! stub unless the `ble` feature is enabled.

use codeisland_domain::{AgentEventType, AgentKind, SessionStatus};

pub const SERVICE_UUID: &str = "0000beef-0000-1000-8000-00805f9b34fb";
pub const WRITE_CHARACTERISTIC_UUID: &str = "0000beef-0001-1000-8000-00805f9b34fb";
pub const NOTIFY_CHARACTERISTIC_UUID: &str = "0000beef-0002-1000-8000-00805f9b34fb";
pub const ADVERTISED_DEVICE_NAME: &str = "Buddy";

pub const MAX_TOOL_NAME_BYTES: usize = 17;
pub const MAX_WORKSPACE_NAME_BYTES: usize = 18;
pub const MAX_MESSAGE_PREVIEW_BYTES: usize = 16;
pub const MAX_MODEL_NAME_BYTES: usize = 18;
pub const MAX_TOOL_HISTORY_NAME_BYTES: usize = 11;
pub const HOST_ID_LENGTH: usize = 6;

pub const MAX_FRAME_BYTES: usize = 3 + MAX_TOOL_NAME_BYTES;
pub const MAX_WORKSPACE_FRAME_BYTES: usize = 2 + MAX_WORKSPACE_NAME_BYTES;
pub const MAX_MESSAGE_PREVIEW_FRAME_BYTES: usize = 4 + MAX_MESSAGE_PREVIEW_BYTES;

pub const BRIGHTNESS_MARKER: u8 = 0xFE;
pub const ORIENTATION_MARKER: u8 = 0xFD;
pub const WORKSPACE_MARKER: u8 = 0xFC;
pub const MESSAGE_PREVIEW_MARKER: u8 = 0xFB;
pub const MODEL_MARKER: u8 = 0xF9;
pub const STATS_MARKER: u8 = 0xFA;
pub const SUBAGENT_MARKER: u8 = 0xF8;
pub const EVENT_MARKER: u8 = 0xF7;
pub const TIME_HINT_MARKER: u8 = 0xF6;
pub const TOOL_HISTORY_MARKER: u8 = 0xF5;

pub const APPROVE_CURRENT_PERMISSION: u8 = 0xF0;
pub const DENY_CURRENT_PERMISSION: u8 = 0xF1;
pub const SKIP_CURRENT_QUESTION: u8 = 0xF2;

pub const PAIR_REQUEST_MARKER: u8 = 0xE0;
pub const UNPAIR_MARKER: u8 = 0xE1;

pub const PAIR_ACCEPTED: u8 = 0xE0;
pub const PAIR_REJECTED: u8 = 0xE1;
pub const PAIR_PENDING: u8 = 0xE2;

pub const MIN_BRIGHTNESS: u8 = 10;
pub const MAX_BRIGHTNESS: u8 = 100;
pub const DEFAULT_BRIGHTNESS: u8 = 70;

/// Agent status codes (`byte[1]` of an agent frame). 0x01 is the processing
/// status, not a frame marker: agent frames are identified by a first byte in
/// 0x00..=0x0F (the mascot slot).
pub const STATUS_IDLE: u8 = 0x00;
pub const STATUS_PROCESSING: u8 = 0x01;
pub const STATUS_RUNNING: u8 = 0x02;
pub const STATUS_WAITING_APPROVAL: u8 = 0x03;
pub const STATUS_WAITING_QUESTION: u8 = 0x04;

/// Firmware leaves AGENT mode after 60 s without a write; 5 s is the
/// recommended sync interval.
pub const SYNC_INTERVAL_SECS: u64 = 5;

/// Namespace mirroring the macOS `ESP32Protocol` enum. All methods are pure
/// byte-level encoders/decoders and work without the `ble` feature.
pub struct BuddyProtocol;

impl BuddyProtocol {
    pub fn agent(mascot: MascotId, status: MascotStatus, tool_name: Option<&str>) -> Vec<u8> {
        agent_frame(mascot, status, tool_name)
    }

    pub fn workspace(name: Option<&str>) -> Vec<u8> {
        workspace_frame(name)
    }

    pub fn preview(index: u8, total: u8, is_user: bool, text: Option<&str>) -> Vec<u8> {
        preview_frame(index, total, is_user, text)
    }

    pub fn brightness(percent: u8) -> Vec<u8> {
        brightness_frame(percent)
    }

    pub fn orientation(orientation: BuddyScreenOrientation) -> Vec<u8> {
        orientation_frame(orientation)
    }

    pub fn model(model: Option<&str>) -> Vec<u8> {
        model_frame(model)
    }

    pub fn stats(
        active_sessions: u8,
        total_sessions: u8,
        tool_calls: u16,
        session_minutes: u8,
    ) -> Vec<u8> {
        stats_frame(active_sessions, total_sessions, tool_calls, session_minutes)
    }

    pub fn subagent(count: u8) -> Vec<u8> {
        subagent_frame(count)
    }

    pub fn event(event_id: u8) -> Vec<u8> {
        event_frame(event_id)
    }

    pub fn time_hint(hour: u8) -> Vec<u8> {
        time_hint_frame(hour)
    }

    pub fn tool_history(index: u8, success: bool, tool_name: &str) -> Vec<u8> {
        tool_history_frame(index, success, tool_name)
    }

    pub fn tool_history_clear() -> Vec<u8> {
        tool_history_clear_frame()
    }

    pub fn pair_request(host_id: &[u8]) -> Vec<u8> {
        pair_request_frame(host_id)
    }

    pub fn unpair(host_id: &[u8]) -> Vec<u8> {
        unpair_frame(host_id)
    }

    pub fn control(command: BuddyControlCommand) -> Vec<u8> {
        control_command_frame(command)
    }

    pub fn decode(bytes: &[u8]) -> Option<BuddyFrame> {
        BuddyFrame::decode(bytes)
    }

    pub fn decode_uplink(payload: &[u8]) -> Option<BuddyUplinkEvent> {
        BuddyUplinkEvent::decode(payload)
    }
}

/// Mascot slot on the Buddy (0..15). Raw values must stay below 0xE0 because
/// the pairing opcodes occupy 0xE0..=0xEF on the uplink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MascotId {
    Claude = 0,
    Codex = 1,
    Gemini = 2,
    Cursor = 3,
    Copilot = 4,
    Trae = 5,
    Qoder = 6,
    Droid = 7,
    CodeBuddy = 8,
    StepFun = 9,
    OpenCode = 10,
    Qwen = 11,
    Antigravity = 12,
    WorkBuddy = 13,
    Hermes = 14,
    Kimi = 15,
}

impl MascotId {
    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            0 => Self::Claude,
            1 => Self::Codex,
            2 => Self::Gemini,
            3 => Self::Cursor,
            4 => Self::Copilot,
            5 => Self::Trae,
            6 => Self::Qoder,
            7 => Self::Droid,
            8 => Self::CodeBuddy,
            9 => Self::StepFun,
            10 => Self::OpenCode,
            11 => Self::Qwen,
            12 => Self::Antigravity,
            13 => Self::WorkBuddy,
            14 => Self::Hermes,
            15 => Self::Kimi,
            _ => return None,
        })
    }

    pub fn raw(self) -> u8 {
        self as u8
    }

    pub fn source_name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
            Self::Copilot => "copilot",
            Self::Trae => "trae",
            Self::Qoder => "qoder",
            Self::Droid => "droid",
            Self::CodeBuddy => "codebuddy",
            Self::StepFun => "stepfun",
            Self::OpenCode => "opencode",
            Self::Qwen => "qwen",
            Self::Antigravity => "antigravity",
            Self::WorkBuddy => "workbuddy",
            Self::Hermes => "hermes",
            Self::Kimi => "kimi",
        }
    }

    /// Fold a CodeIsland source tag (including aliases) onto the 16 firmware
    /// slots. Antigravity folds onto Gemini and MiMo onto OpenCode when the
    /// dedicated slot is already used, mirroring the macOS implementation.
    pub fn from_source(source: &str) -> Option<Self> {
        Some(match source.to_ascii_lowercase().as_str() {
            "claude" | "claude-code" | "claude_code" => Self::Claude,
            "codex" => Self::Codex,
            "gemini" | "gemini-cli" | "google-antigravity" => Self::Gemini,
            "cursor" | "cursor-cli" => Self::Cursor,
            "copilot" => Self::Copilot,
            "trae" | "traecn" | "traecli" => Self::Trae,
            "qoder" | "qoder-cli" | "qodercn" | "qoderwork" => Self::Qoder,
            "droid" | "factory" => Self::Droid,
            "codebuddy" | "codybuddycn" => Self::CodeBuddy,
            "stepfun" => Self::StepFun,
            "opencode" | "mimo" => Self::OpenCode,
            "qwen" => Self::Qwen,
            "antigravity" => Self::Antigravity,
            "workbuddy" => Self::WorkBuddy,
            "hermes" => Self::Hermes,
            "kimi" => Self::Kimi,
            _ => return None,
        })
    }

    pub fn from_agent(agent: AgentKind) -> Option<Self> {
        match agent {
            AgentKind::Claude => Some(Self::Claude),
            AgentKind::Codex => Some(Self::Codex),
            AgentKind::Gemini => Some(Self::Gemini),
            AgentKind::Cursor => Some(Self::Cursor),
            AgentKind::Copilot => Some(Self::Copilot),
            AgentKind::Trae | AgentKind::TraeCn | AgentKind::TraeCli => Some(Self::Trae),
            AgentKind::Qoder | AgentKind::QoderCn | AgentKind::QoderWork => Some(Self::Qoder),
            AgentKind::Factory => Some(Self::Droid),
            AgentKind::CodeBuddy | AgentKind::CodyBuddyCn => Some(Self::CodeBuddy),
            AgentKind::StepFun => Some(Self::StepFun),
            AgentKind::OpenCode | AgentKind::MiMo => Some(Self::OpenCode),
            AgentKind::Qwen => Some(Self::Qwen),
            AgentKind::Antigravity => Some(Self::Antigravity),
            AgentKind::WorkBuddy => Some(Self::WorkBuddy),
            AgentKind::Hermes => Some(Self::Hermes),
            AgentKind::Kimi => Some(Self::Kimi),
            _ => None,
        }
    }
}

/// On-wire status code: 0 SLEEP, 1/2 WORK (tool name is drawn), 3 ALERT,
/// 4 QUESTION.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MascotStatus {
    Idle = 0,
    Processing = 1,
    Running = 2,
    WaitingApproval = 3,
    WaitingQuestion = 4,
}

impl MascotStatus {
    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            0 => Self::Idle,
            1 => Self::Processing,
            2 => Self::Running,
            3 => Self::WaitingApproval,
            4 => Self::WaitingQuestion,
            _ => return None,
        })
    }

    pub fn raw(self) -> u8 {
        self as u8
    }

    pub fn from_session_status(status: SessionStatus) -> Self {
        match status {
            SessionStatus::Idle => Self::Idle,
            SessionStatus::Starting => Self::Processing,
            SessionStatus::Running => Self::Running,
            SessionStatus::WaitingPermission => Self::WaitingApproval,
            SessionStatus::WaitingAnswer => Self::WaitingQuestion,
            SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled => {
                Self::Idle
            }
        }
    }

    pub fn from_event_type(event: AgentEventType) -> Self {
        match event {
            AgentEventType::SessionStart | AgentEventType::UserPrompt => Self::Processing,
            AgentEventType::ToolStart => Self::Running,
            AgentEventType::PermissionRequest => Self::WaitingApproval,
            AgentEventType::Question => Self::WaitingQuestion,
            _ => Self::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BuddyScreenOrientation {
    Up = 0,
    Down = 1,
}

impl BuddyScreenOrientation {
    pub fn wire_value(self) -> u8 {
        self as u8
    }

    pub fn from_wire(value: u8) -> Self {
        if value == 1 { Self::Down } else { Self::Up }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BuddyControlCommand {
    ApproveCurrentPermission = APPROVE_CURRENT_PERMISSION,
    DenyCurrentPermission = DENY_CURRENT_PERMISSION,
    SkipCurrentQuestion = SKIP_CURRENT_QUESTION,
}

impl BuddyControlCommand {
    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            APPROVE_CURRENT_PERMISSION => Self::ApproveCurrentPermission,
            DENY_CURRENT_PERMISSION => Self::DenyCurrentPermission,
            SKIP_CURRENT_QUESTION => Self::SkipCurrentQuestion,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BuddyPairResponse {
    Accepted = PAIR_ACCEPTED,
    Rejected = PAIR_REJECTED,
    Pending = PAIR_PENDING,
}

impl BuddyPairResponse {
    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            PAIR_ACCEPTED => Self::Accepted,
            PAIR_REJECTED => Self::Rejected,
            PAIR_PENDING => Self::Pending,
            _ => return None,
        })
    }
}

/// Decoded uplink notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuddyUplinkEvent {
    /// A mascot slot was tapped: focus that agent.
    Focus(MascotId),
    Command(BuddyControlCommand),
    PairResponse(BuddyPairResponse),
}

impl BuddyUplinkEvent {
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let first = *payload.first()?;
        // Pair responses are checked first; mascot raw values are 0..=15 and
        // therefore never collide with the 0xE0..=0xE2 reserved range.
        if let Some(response) = BuddyPairResponse::from_raw(first) {
            return Some(Self::PairResponse(response));
        }
        if let Some(mascot) = MascotId::from_raw(first) {
            return Some(Self::Focus(mascot));
        }
        BuddyControlCommand::from_raw(first).map(Self::Command)
    }
}

/// A decoded downlink frame. Useful for diagnostics and round-trip tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuddyFrame {
    Agent {
        mascot: MascotId,
        status: MascotStatus,
        tool_name: String,
    },
    Workspace(String),
    Preview {
        index: u8,
        total: u8,
        is_user: bool,
        text: String,
    },
    Brightness(u8),
    Orientation(BuddyScreenOrientation),
    Model(String),
    Stats {
        active_sessions: u8,
        total_sessions: u8,
        tool_calls: u16,
        session_minutes: u8,
    },
    Subagent(u8),
    Event(u8),
    TimeHint(u8),
    ToolHistory {
        index: u8,
        success: bool,
        tool_name: String,
    },
    ToolHistoryClear,
    PairRequest([u8; HOST_ID_LENGTH]),
    Unpair([u8; HOST_ID_LENGTH]),
    Control(BuddyControlCommand),
}

// ---------------------------------------------------------------------------
// Frame encoders
// ---------------------------------------------------------------------------

/// Agent frame: `[sourceId, statusId, toolLen, toolName...]`.
pub fn agent_frame(mascot: MascotId, status: MascotStatus, tool_name: Option<&str>) -> Vec<u8> {
    let tool = tool_name.unwrap_or("");
    let bytes = truncate_utf8(tool, MAX_TOOL_NAME_BYTES);
    let mut frame = Vec::with_capacity(3 + bytes.len());
    frame.push(mascot.raw());
    frame.push(status.raw());
    frame.push(bytes.len() as u8);
    frame.extend_from_slice(&bytes);
    frame
}

/// Workspace frame: `[0xFC, len, workspace...]`.
pub fn workspace_frame(name: Option<&str>) -> Vec<u8> {
    let bytes = truncate_utf8(name.unwrap_or("").trim(), MAX_WORKSPACE_NAME_BYTES);
    let mut frame = Vec::with_capacity(2 + bytes.len());
    frame.push(WORKSPACE_MARKER);
    frame.push(bytes.len() as u8);
    frame.extend_from_slice(&bytes);
    frame
}

/// Message preview frame:
/// `[0xFB, index, total, (isUser << 7) | len, text...]`.
pub fn preview_frame(index: u8, total: u8, is_user: bool, text: Option<&str>) -> Vec<u8> {
    let bytes = truncate_utf8(text.unwrap_or(""), MAX_MESSAGE_PREVIEW_BYTES);
    let flags = (if is_user { 0x80 } else { 0x00 }) | bytes.len() as u8;
    let mut frame = Vec::with_capacity(4 + bytes.len());
    frame.push(MESSAGE_PREVIEW_MARKER);
    frame.push(index);
    frame.push(total);
    frame.push(flags);
    frame.extend_from_slice(&bytes);
    frame
}

/// Brightness frame: `[0xFE, percent]`, clamped to 10..=100.
pub fn brightness_frame(percent: u8) -> Vec<u8> {
    vec![BRIGHTNESS_MARKER, percent.clamp(MIN_BRIGHTNESS, MAX_BRIGHTNESS)]
}

/// Orientation frame: `[0xFD, 0|1]`.
pub fn orientation_frame(orientation: BuddyScreenOrientation) -> Vec<u8> {
    vec![ORIENTATION_MARKER, orientation.wire_value()]
}

/// Model frame: `[0xF9, len, model...]`.
pub fn model_frame(model: Option<&str>) -> Vec<u8> {
    let bytes = truncate_utf8(model.unwrap_or(""), MAX_MODEL_NAME_BYTES);
    let mut frame = Vec::with_capacity(2 + bytes.len());
    frame.push(MODEL_MARKER);
    frame.push(bytes.len() as u8);
    frame.extend_from_slice(&bytes);
    frame
}

/// Stats frame: `[0xFA, active, total, toolHi, toolLo, minutes]`.
pub fn stats_frame(active_sessions: u8, total_sessions: u8, tool_calls: u16, session_minutes: u8) -> Vec<u8> {
    vec![
        STATS_MARKER,
        active_sessions,
        total_sessions,
        (tool_calls >> 8) as u8,
        (tool_calls & 0xFF) as u8,
        session_minutes,
    ]
}

/// Subagent frame: `[0xF8, count]`, count clamped to 0..=15.
pub fn subagent_frame(count: u8) -> Vec<u8> {
    vec![SUBAGENT_MARKER, count.min(15)]
}

/// Event frame: `[0xF7, eventId]`.
pub fn event_frame(event_id: u8) -> Vec<u8> {
    vec![EVENT_MARKER, event_id]
}

/// Time hint frame: `[0xF6, hour]`, hour clamped to 0..=23.
pub fn time_hint_frame(hour: u8) -> Vec<u8> {
    vec![TIME_HINT_MARKER, hour.min(23)]
}

/// Tool history frame: `[0xF5, index, (success << 7) | len, name...]`.
pub fn tool_history_frame(index: u8, success: bool, tool_name: &str) -> Vec<u8> {
    let bytes = truncate_utf8(tool_name, MAX_TOOL_HISTORY_NAME_BYTES);
    let flags = (if success { 0x80 } else { 0x00 }) | bytes.len() as u8;
    let mut frame = Vec::with_capacity(3 + bytes.len());
    frame.push(TOOL_HISTORY_MARKER);
    frame.push(index);
    frame.push(flags);
    frame.extend_from_slice(&bytes);
    frame
}

/// Clear the tool history: `[0xF5, 0, 0]`.
pub fn tool_history_clear_frame() -> Vec<u8> {
    vec![TOOL_HISTORY_MARKER, 0, 0]
}

/// Pair request: `[0xE0, hostId(6)]`, zero-padded.
pub fn pair_request_frame(host_id: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + HOST_ID_LENGTH);
    frame.push(PAIR_REQUEST_MARKER);
    frame.extend_from_slice(&pad_host_id(host_id));
    frame
}

/// Unpair: `[0xE1, hostId(6)]`, zero-padded.
pub fn unpair_frame(host_id: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1 + HOST_ID_LENGTH);
    frame.push(UNPAIR_MARKER);
    frame.extend_from_slice(&pad_host_id(host_id));
    frame
}

/// Control opcode (uplink): a single byte.
pub fn control_command_frame(command: BuddyControlCommand) -> Vec<u8> {
    vec![command as u8]
}

// ---------------------------------------------------------------------------
// Frame decoder
// ---------------------------------------------------------------------------

impl BuddyFrame {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let first = *bytes.first()?;
        // Mascot raw values are 0..=15, so an agent frame is unambiguous.
        if first <= 15 {
            let mascot = MascotId::from_raw(first)?;
            let status = MascotStatus::from_raw(*bytes.get(1)?)?;
            let len = *bytes.get(2)? as usize;
            let tool = bytes.get(3..3 + len)?;
            return Some(Self::Agent {
                mascot,
                status,
                tool_name: String::from_utf8_lossy(tool).into_owned(),
            });
        }
        match first {
            WORKSPACE_MARKER => {
                let len = *bytes.get(1)? as usize;
                Some(Self::Workspace(
                    String::from_utf8_lossy(bytes.get(2..2 + len)?).into_owned(),
                ))
            }
            MESSAGE_PREVIEW_MARKER => {
                let index = *bytes.get(1)?;
                let total = *bytes.get(2)?;
                let flags = *bytes.get(3)?;
                let len = (flags & 0x7F) as usize;
                Some(Self::Preview {
                    index,
                    total,
                    is_user: flags & 0x80 != 0,
                    text: String::from_utf8_lossy(bytes.get(4..4 + len)?).into_owned(),
                })
            }
            BRIGHTNESS_MARKER => Some(Self::Brightness(*bytes.get(1)?)),
            ORIENTATION_MARKER => Some(Self::Orientation(
                BuddyScreenOrientation::from_wire(*bytes.get(1)?),
            )),
            MODEL_MARKER => {
                let len = *bytes.get(1)? as usize;
                Some(Self::Model(
                    String::from_utf8_lossy(bytes.get(2..2 + len)?).into_owned(),
                ))
            }
            STATS_MARKER => Some(Self::Stats {
                active_sessions: *bytes.get(1)?,
                total_sessions: *bytes.get(2)?,
                tool_calls: u16::from_be_bytes([*bytes.get(3)?, *bytes.get(4)?]),
                session_minutes: *bytes.get(5)?,
            }),
            SUBAGENT_MARKER => Some(Self::Subagent(*bytes.get(1)?)),
            EVENT_MARKER => Some(Self::Event(*bytes.get(1)?)),
            TIME_HINT_MARKER => Some(Self::TimeHint(*bytes.get(1)?)),
            TOOL_HISTORY_MARKER => {
                let index = *bytes.get(1)?;
                let flags = *bytes.get(2)?;
                if index == 0 && flags == 0 {
                    return Some(Self::ToolHistoryClear);
                }
                let len = (flags & 0x7F) as usize;
                Some(Self::ToolHistory {
                    index,
                    success: flags & 0x80 != 0,
                    tool_name: String::from_utf8_lossy(bytes.get(3..3 + len)?).into_owned(),
                })
            }
            PAIR_REQUEST_MARKER => Some(Self::PairRequest(host_id(bytes.get(1..)?)?)),
            UNPAIR_MARKER => Some(Self::Unpair(host_id(bytes.get(1..)?)?)),
            APPROVE_CURRENT_PERMISSION | DENY_CURRENT_PERMISSION | SKIP_CURRENT_QUESTION => {
                BuddyControlCommand::from_raw(first).map(Self::Control)
            }
            _ => None,
        }
    }
}

/// A coherent Buddy screen state. [`BuddyState::frames`] returns the frame
/// sequence the manager writes on every sync tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuddyState {
    pub mascot: MascotId,
    pub status: MascotStatus,
    pub tool_name: Option<String>,
    pub workspace: Option<String>,
    pub model: Option<String>,
    pub active_sessions: u8,
    pub total_sessions: u8,
    pub tool_calls: u16,
    pub session_minutes: u8,
    pub subagents: u8,
    pub preview: Option<(u8, u8, bool, String)>,
    pub history: Vec<(u8, bool, String)>,
    pub hour: Option<u8>,
}

impl BuddyState {
    pub fn new(mascot: MascotId) -> Self {
        Self {
            mascot,
            status: MascotStatus::Idle,
            tool_name: None,
            workspace: None,
            model: None,
            active_sessions: 0,
            total_sessions: 0,
            tool_calls: 0,
            session_minutes: 0,
            subagents: 0,
            preview: None,
            history: Vec::new(),
            hour: None,
        }
    }

    pub fn frames(&self) -> Vec<Vec<u8>> {
        let mut frames = vec![
            agent_frame(self.mascot, self.status, self.tool_name.as_deref()),
            workspace_frame(self.workspace.as_deref()),
            model_frame(self.model.as_deref()),
            stats_frame(
                self.active_sessions,
                self.total_sessions,
                self.tool_calls,
                self.session_minutes,
            ),
            subagent_frame(self.subagents),
        ];
        if let Some((index, total, is_user, text)) = &self.preview {
            frames.push(preview_frame(*index, *total, *is_user, Some(text.as_str())));
        }
        for (index, success, name) in &self.history {
            frames.push(tool_history_frame(*index, *success, name.as_str()));
        }
        if let Some(hour) = self.hour {
            frames.push(time_hint_frame(hour));
        }
        frames
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BuddyError {
    #[error("BLE support is not compiled into this build")]
    Unsupported,
    #[error("Bluetooth error: {0}")]
    Bluetooth(String),
    #[error("no Buddy device found")]
    NotFound,
    #[error("Buddy is not connected")]
    NotConnected,
    #[error("Buddy protocol error: {0}")]
    Protocol(String),
}

/// Commands the UI can receive from the Buddy's buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuddyCommand {
    Approve,
    Deny,
    Skip,
    /// A mascot was tapped. The UI interprets consecutive focus events as
    /// next/previous navigation across the session list.
    Focus(MascotId),
}

impl BuddyUplinkEvent {
    /// Button events become UI commands; pairing verdicts are not commands.
    pub fn into_command(self) -> Option<BuddyCommand> {
        match self {
            Self::Focus(mascot) => Some(BuddyCommand::Focus(mascot)),
            Self::Command(BuddyControlCommand::ApproveCurrentPermission) => {
                Some(BuddyCommand::Approve)
            }
            Self::Command(BuddyControlCommand::DenyCurrentPermission) => Some(BuddyCommand::Deny),
            Self::Command(BuddyControlCommand::SkipCurrentQuestion) => Some(BuddyCommand::Skip),
            Self::PairResponse(_) => None,
        }
    }
}

/// Random, stable-per-install 6-byte host identifier used by the pairing
/// handshake.
pub fn random_host_id() -> [u8; HOST_ID_LENGTH] {
    crate::random_bytes::<HOST_ID_LENGTH>()
}

#[cfg(feature = "ble")]
mod manager {
    use std::sync::Arc;
    use std::time::Duration;

    use btleplug::api::{
        Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
        WriteType,
    };
    use btleplug::platform::{Adapter, Manager, Peripheral};
    use futures_util::StreamExt;
    use tokio::sync::{mpsc, watch};

    use super::*;

    /// Live BLE connection to a Buddy device.
    pub struct BuddyManager {
        adapter: Adapter,
        peripheral: Option<Peripheral>,
        write_characteristic: Option<Characteristic>,
        notify_characteristic: Option<Characteristic>,
        host_id: [u8; HOST_ID_LENGTH],
        state: BuddyState,
    }

    impl BuddyManager {
        pub async fn new() -> Result<Self, BuddyError> {
            let manager = Manager::new()
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;
            let adapter = manager
                .adapters()
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?
                .into_iter()
                .next()
                .ok_or_else(|| BuddyError::Bluetooth("no Bluetooth adapter".into()))?;
            Ok(Self {
                adapter,
                peripheral: None,
                write_characteristic: None,
                notify_characteristic: None,
                host_id: random_host_id(),
                state: BuddyState::new(MascotId::Claude),
            })
        }

        pub fn host_id(&self) -> [u8; HOST_ID_LENGTH] {
            self.host_id
        }

        pub fn state(&self) -> &BuddyState {
            &self.state
        }

        pub fn is_connected(&self) -> bool {
            self.peripheral.is_some()
        }

        /// Scans for a device advertising the Buddy service (or named
        /// `Buddy`), connects, and caches the write/notify characteristics.
        pub async fn scan_and_connect(&mut self) -> Result<(), BuddyError> {
            self.adapter
                .start_scan(ScanFilter::default())
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;
            // Give the controller a moment to report advertisements.
            tokio::time::sleep(Duration::from_secs(2)).await;
            let peripherals = self
                .adapter
                .peripherals()
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;
            let mut found = None;
            for peripheral in peripherals {
                let properties = peripheral
                    .properties()
                    .await
                    .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;
                let matches = properties
                    .map(|properties| {
                        properties.local_name.as_deref() == Some(ADVERTISED_DEVICE_NAME)
                            || properties
                                .services
                                .iter()
                                .any(|uuid| uuid.to_string() == SERVICE_UUID)
                    })
                    .unwrap_or(false);
                if matches {
                    found = Some(peripheral);
                    break;
                }
            }
            let _ = self.adapter.stop_scan().await;
            let peripheral = found.ok_or(BuddyError::NotFound)?;

            peripheral
                .connect()
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;
            peripheral
                .discover_services()
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;

            let mut write = None;
            let mut notify = None;
            for characteristic in peripheral.characteristics() {
                let uuid = characteristic.uuid.to_string();
                if uuid == WRITE_CHARACTERISTIC_UUID
                    && (characteristic.properties.contains(CharPropFlags::WRITE)
                        || characteristic
                            .properties
                            .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE))
                {
                    write = Some(characteristic);
                } else if uuid == NOTIFY_CHARACTERISTIC_UUID
                    && characteristic.properties.contains(CharPropFlags::NOTIFY)
                {
                    notify = Some(characteristic);
                }
            }
            let write = write.ok_or_else(|| {
                BuddyError::Protocol("write characteristic not found".into())
            })?;
            let notify = notify.ok_or_else(|| {
                BuddyError::Protocol("notify characteristic not found".into())
            })?;
            peripheral
                .subscribe(&notify)
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;

            self.peripheral = Some(peripheral);
            self.write_characteristic = Some(write);
            self.notify_characteristic = Some(notify);
            Ok(())
        }

        /// Sends the pairing handshake. The firmware's verdict arrives on the
        /// notification stream as [`BuddyUplinkEvent::PairResponse`].
        pub async fn pair(&self) -> Result<(), BuddyError> {
            self.write(&pair_request_frame(&self.host_id)).await
        }

        pub async fn unpair(&self) -> Result<(), BuddyError> {
            self.write(&unpair_frame(&self.host_id)).await
        }

        async fn write(&self, frame: &[u8]) -> Result<(), BuddyError> {
            let peripheral = self.peripheral.as_ref().ok_or(BuddyError::NotConnected)?;
            let characteristic = self
                .write_characteristic
                .as_ref()
                .ok_or(BuddyError::NotConnected)?;
            peripheral
                .write(characteristic, frame, WriteType::WithoutResponse)
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))
        }

        pub async fn write_state(&mut self, state: &BuddyState) -> Result<(), BuddyError> {
            self.state = state.clone();
            for frame in state.frames() {
                self.write(&frame).await?;
            }
            Ok(())
        }

        /// Writes on every state change and at least every 5 s (the firmware
        /// leaves AGENT mode after 60 s), while forwarding button events.
        pub async fn run(
            &mut self,
            mut state: watch::Receiver<BuddyState>,
            commands: mpsc::Sender<BuddyCommand>,
        ) -> Result<(), BuddyError> {
            let peripheral = self
                .peripheral
                .clone()
                .ok_or(BuddyError::NotConnected)?;
            let mut notifications = peripheral
                .notifications()
                .await
                .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;
            let mut interval = tokio::time::interval(Duration::from_secs(SYNC_INTERVAL_SECS));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let snapshot = state.borrow().clone();
                        self.write_state(&snapshot).await?;
                    }
                    changed = state.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        let snapshot = state.borrow().clone();
                        self.write_state(&snapshot).await?;
                    }
                    notification = notifications.next() => {
                        let Some(notification) = notification else { break };
                        if let Some(event) = BuddyUplinkEvent::decode(&notification.value) {
                            if let Some(command) = event.into_command() {
                                let _ = commands.send(command).await;
                            }
                        }
                    }
                }
            }
            Ok(())
        }

        pub async fn disconnect(&mut self) -> Result<(), BuddyError> {
            if let (Some(peripheral), Some(notify)) = (
                self.peripheral.as_ref(),
                self.notify_characteristic.as_ref(),
            ) {
                let _ = peripheral.unsubscribe(notify).await;
            }
            if let Some(peripheral) = self.peripheral.take() {
                peripheral
                    .disconnect()
                    .await
                    .map_err(|error| BuddyError::Bluetooth(error.to_string()))?;
            }
            self.write_characteristic = None;
            self.notify_characteristic = None;
            Ok(())
        }
    }

    /// Keeps `Arc` usable for callers that share the manager across tasks.
    impl BuddyManager {
        pub fn into_shared(self) -> Arc<tokio::sync::Mutex<Self>> {
            Arc::new(tokio::sync::Mutex::new(self))
        }
    }
}

#[cfg(not(feature = "ble"))]
mod manager {
    use tokio::sync::{mpsc, watch};

    use super::*;

    /// Stub used when the `ble` feature is off.
    pub struct BuddyManager;

    impl BuddyManager {
        pub async fn new() -> Result<Self, BuddyError> {
            Err(BuddyError::Unsupported)
        }

        pub fn host_id(&self) -> [u8; HOST_ID_LENGTH] {
            [0; HOST_ID_LENGTH]
        }

        pub fn state(&self) -> &BuddyState {
            // Unreachable in practice: the stub cannot be constructed.
            static EMPTY: std::sync::OnceLock<BuddyState> = std::sync::OnceLock::new();
            EMPTY.get_or_init(|| BuddyState::new(MascotId::Claude))
        }

        pub fn is_connected(&self) -> bool {
            false
        }

        pub async fn scan_and_connect(&mut self) -> Result<(), BuddyError> {
            Err(BuddyError::Unsupported)
        }

        pub async fn pair(&self) -> Result<(), BuddyError> {
            Err(BuddyError::Unsupported)
        }

        pub async fn unpair(&self) -> Result<(), BuddyError> {
            Err(BuddyError::Unsupported)
        }

        pub async fn write_state(&mut self, _state: &BuddyState) -> Result<(), BuddyError> {
            Err(BuddyError::Unsupported)
        }

        pub async fn run(
            &mut self,
            _state: watch::Receiver<BuddyState>,
            _commands: mpsc::Sender<BuddyCommand>,
        ) -> Result<(), BuddyError> {
            Err(BuddyError::Unsupported)
        }

        pub async fn disconnect(&mut self) -> Result<(), BuddyError> {
            Err(BuddyError::Unsupported)
        }
    }
}

pub use manager::BuddyManager;

fn truncate_utf8(text: &str, limit: usize) -> Vec<u8> {
    let bytes = text.as_bytes();
    bytes[..bytes.len().min(limit)].to_vec()
}

fn pad_host_id(host_id: &[u8]) -> [u8; HOST_ID_LENGTH] {
    let mut out = [0u8; HOST_ID_LENGTH];
    let len = host_id.len().min(HOST_ID_LENGTH);
    out[..len].copy_from_slice(&host_id[..len]);
    out
}

fn host_id(bytes: &[u8]) -> Option<[u8; HOST_ID_LENGTH]> {
    if bytes.len() < HOST_ID_LENGTH {
        return None;
    }
    let mut out = [0u8; HOST_ID_LENGTH];
    out.copy_from_slice(&bytes[..HOST_ID_LENGTH]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_frame_layout() {
        assert_eq!(
            agent_frame(MascotId::Claude, MascotStatus::Processing, Some("Bash")),
            vec![0, 1, 4, b'B', b'a', b's', b'h']
        );
        assert_eq!(
            agent_frame(MascotId::Kimi, MascotStatus::WaitingApproval, None),
            vec![15, 3, 0]
        );
    }

    #[test]
    fn agent_frame_truncates_tool_to_17_bytes() {
        let long = "abcdefghijklmnopqrstuvwxyz";
        let frame = agent_frame(MascotId::Codex, MascotStatus::Running, Some(long));
        assert_eq!(frame.len(), 3 + 17);
        assert_eq!(frame[2], 17);
        assert_eq!(&frame[3..], b"abcdefghijklmnopq");
    }

    #[test]
    fn workspace_frame_layout() {
        assert_eq!(
            workspace_frame(Some("vibe-notch")),
            {
                let mut expected = vec![WORKSPACE_MARKER, 10];
                expected.extend_from_slice(b"vibe-notch");
                expected
            }
        );
        assert_eq!(workspace_frame(None), vec![WORKSPACE_MARKER, 0]);
    }

    #[test]
    fn preview_frame_layout() {
        let frame = preview_frame(1, 3, true, Some("hello"));
        assert_eq!(frame, vec![0xFB, 1, 3, 0x80 | 5, b'h', b'e', b'l', b'l', b'o']);
        let frame = preview_frame(0, 1, false, Some("hi"));
        assert_eq!(frame, vec![0xFB, 0, 1, 2, b'h', b'i']);
    }

    #[test]
    fn brightness_and_orientation_layout() {
        assert_eq!(brightness_frame(70), vec![0xFE, 70]);
        assert_eq!(brightness_frame(0), vec![0xFE, 10]);
        assert_eq!(brightness_frame(255), vec![0xFE, 100]);
        assert_eq!(
            orientation_frame(BuddyScreenOrientation::Up),
            vec![0xFD, 0]
        );
        assert_eq!(
            orientation_frame(BuddyScreenOrientation::Down),
            vec![0xFD, 1]
        );
    }

    #[test]
    fn model_frame_layout() {
        assert_eq!(model_frame(Some("opus")), vec![0xF9, 4, b'o', b'p', b'u', b's']);
        assert_eq!(model_frame(None), vec![0xF9, 0]);
    }

    #[test]
    fn stats_frame_layout() {
        assert_eq!(
            stats_frame(1, 2, 258, 3),
            vec![0xFA, 1, 2, 0x01, 0x02, 3]
        );
        assert_eq!(stats_frame(0, 0, 0, 0), vec![0xFA, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn small_frames_layout() {
        assert_eq!(subagent_frame(2), vec![0xF8, 2]);
        assert_eq!(subagent_frame(99), vec![0xF8, 15]);
        assert_eq!(event_frame(3), vec![0xF7, 3]);
        assert_eq!(time_hint_frame(23), vec![0xF6, 23]);
        assert_eq!(time_hint_frame(99), vec![0xF6, 23]);
    }

    #[test]
    fn history_frames_layout() {
        assert_eq!(
            tool_history_frame(0, true, "Read"),
            vec![0xF5, 0, 0x80 | 4, b'R', b'e', b'a', b'd']
        );
        assert_eq!(
            tool_history_frame(2, false, "Bash"),
            vec![0xF5, 2, 4, b'B', b'a', b's', b'h']
        );
        assert_eq!(tool_history_clear_frame(), vec![0xF5, 0, 0]);
    }

    #[test]
    fn pairing_frames_layout() {
        assert_eq!(
            pair_request_frame(&[1, 2, 3, 4, 5, 6]),
            vec![0xE0, 1, 2, 3, 4, 5, 6]
        );
        assert_eq!(
            pair_request_frame(&[1, 2]),
            vec![0xE0, 1, 2, 0, 0, 0, 0]
        );
        assert_eq!(
            unpair_frame(&[1, 2, 3, 4, 5, 6]),
            vec![0xE1, 1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn control_frames_layout() {
        assert_eq!(
            control_command_frame(BuddyControlCommand::ApproveCurrentPermission),
            vec![0xF0]
        );
        assert_eq!(
            control_command_frame(BuddyControlCommand::DenyCurrentPermission),
            vec![0xF1]
        );
        assert_eq!(
            control_command_frame(BuddyControlCommand::SkipCurrentQuestion),
            vec![0xF2]
        );
    }

    #[test]
    fn decodes_round_trip() {
        let frame = agent_frame(MascotId::Gemini, MascotStatus::Running, Some("Edit"));
        assert_eq!(
            BuddyFrame::decode(&frame),
            Some(BuddyFrame::Agent {
                mascot: MascotId::Gemini,
                status: MascotStatus::Running,
                tool_name: "Edit".into(),
            })
        );

        let frame = stats_frame(3, 5, 1024, 12);
        assert_eq!(
            BuddyFrame::decode(&frame),
            Some(BuddyFrame::Stats {
                active_sessions: 3,
                total_sessions: 5,
                tool_calls: 1024,
                session_minutes: 12,
            })
        );

        let frame = preview_frame(2, 4, true, Some("ok"));
        assert_eq!(
            BuddyFrame::decode(&frame),
            Some(BuddyFrame::Preview {
                index: 2,
                total: 4,
                is_user: true,
                text: "ok".into(),
            })
        );

        assert_eq!(
            BuddyFrame::decode(&tool_history_clear_frame()),
            Some(BuddyFrame::ToolHistoryClear)
        );
        assert_eq!(
            BuddyFrame::decode(&pair_request_frame(&[9; 6])),
            Some(BuddyFrame::PairRequest([9; 6]))
        );
        assert_eq!(
            BuddyFrame::decode(&vec![0xF1]),
            Some(BuddyFrame::Control(BuddyControlCommand::DenyCurrentPermission))
        );
        assert_eq!(BuddyFrame::decode(&[0xF5, 0]), None);
    }

    #[test]
    fn uplink_decoding_prefers_pairing_range() {
        assert_eq!(
            BuddyUplinkEvent::decode(&[0xE0]),
            Some(BuddyUplinkEvent::PairResponse(BuddyPairResponse::Accepted))
        );
        assert_eq!(
            BuddyUplinkEvent::decode(&[0xE2]),
            Some(BuddyUplinkEvent::PairResponse(BuddyPairResponse::Pending))
        );
        assert_eq!(
            BuddyUplinkEvent::decode(&[3]),
            Some(BuddyUplinkEvent::Focus(MascotId::Cursor))
        );
        assert_eq!(
            BuddyUplinkEvent::decode(&[0xF2]),
            Some(BuddyUplinkEvent::Command(
                BuddyControlCommand::SkipCurrentQuestion
            ))
        );
        assert_eq!(BuddyUplinkEvent::decode(&[0x10]), None);
        assert_eq!(BuddyUplinkEvent::decode(&[]), None);
    }

    #[test]
    fn commands_map_to_ui_actions() {
        assert_eq!(
            BuddyUplinkEvent::Command(BuddyControlCommand::ApproveCurrentPermission)
                .into_command(),
            Some(BuddyCommand::Approve)
        );
        assert_eq!(
            BuddyUplinkEvent::Command(BuddyControlCommand::DenyCurrentPermission)
                .into_command(),
            Some(BuddyCommand::Deny)
        );
        assert_eq!(
            BuddyUplinkEvent::Focus(MascotId::Qwen).into_command(),
            Some(BuddyCommand::Focus(MascotId::Qwen))
        );
        assert_eq!(
            BuddyUplinkEvent::PairResponse(BuddyPairResponse::Accepted).into_command(),
            None
        );
    }

    #[test]
    fn state_frames_include_expected_markers() {
        let mut state = BuddyState::new(MascotId::Claude);
        state.status = MascotStatus::WaitingApproval;
        state.tool_name = Some("Bash".into());
        state.workspace = Some("proj".into());
        state.model = Some("opus".into());
        state.history.push((0, true, "Read".into()));
        state.hour = Some(9);
        let frames = state.frames();
        assert_eq!(frames[0][0], 0);
        assert_eq!(frames[1][0], WORKSPACE_MARKER);
        assert_eq!(frames[2][0], MODEL_MARKER);
        assert_eq!(frames[3][0], STATS_MARKER);
        assert_eq!(frames[4][0], SUBAGENT_MARKER);
        assert_eq!(frames.last().unwrap()[0], TIME_HINT_MARKER);
    }

    #[test]
    fn protocol_namespace_delegates() {
        let frame = BuddyProtocol::agent(MascotId::Claude, MascotStatus::Processing, Some("Bash"));
        assert_eq!(
            frame,
            agent_frame(MascotId::Claude, MascotStatus::Processing, Some("Bash"))
        );
        assert_eq!(frame[1], STATUS_PROCESSING);
        assert_eq!(
            BuddyProtocol::decode(&frame),
            Some(BuddyFrame::Agent {
                mascot: MascotId::Claude,
                status: MascotStatus::Processing,
                tool_name: "Bash".into(),
            })
        );
        assert_eq!(
            BuddyProtocol::decode_uplink(&[0xF0]),
            Some(BuddyUplinkEvent::Command(
                BuddyControlCommand::ApproveCurrentPermission
            ))
        );
    }

    #[test]
    fn source_and_agent_mapping() {
        assert_eq!(MascotId::from_source("claude"), Some(MascotId::Claude));
        assert_eq!(MascotId::from_source("cursor-cli"), Some(MascotId::Cursor));
        assert_eq!(MascotId::from_source("mimo"), Some(MascotId::OpenCode));
        assert_eq!(MascotId::from_source("unknown"), None);
        assert_eq!(
            MascotId::from_agent(AgentKind::Factory),
            Some(MascotId::Droid)
        );
        assert_eq!(
            MascotId::from_agent(AgentKind::CodyBuddyCn),
            Some(MascotId::CodeBuddy)
        );
        assert_eq!(MascotId::from_agent(AgentKind::Unknown), None);
    }
}
