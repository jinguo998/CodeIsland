import Foundation

// MARK: - Status mapping helpers (pure)

public enum AiWorkStatusMapper {
    /// Map a daemon stream event name onto CodeIsland's AgentStatus.
    /// Returns nil when the event should not change status (e.g. diagnostics).
    public static func status(forEventName name: String) -> AgentStatus? {
        switch name {
        case "stream.approval_required",
             "stream.plan_confirmation_required":
            return .waitingApproval
        case "stream.question_required":
            return .waitingQuestion
        case "stream.tool_call":
            return .running
        case "stream.tool_result",
             "stream.approval_resolved",
             "stream.plan_confirmation_resolved",
             "stream.question_resolved":
            return .processing
        case "stream.started",
             "stream.progress",
             "stream.thinking_delta",
             "stream.text_delta",
             "stream.ack",
             "stream.fallback_started":
            return .processing
        case "stream.completed",
             "stream.failed",
             "stream.aborted":
            return .idle
        default:
            return nil
        }
    }

    /// Whether a terminal event should enqueue a completion card.
    public static func isTerminalEvent(_ name: String) -> Bool {
        switch name {
        case "stream.completed", "stream.failed", "stream.aborted":
            return true
        default:
            return false
        }
    }

    /// Extract a short tool label from a `stream.tool_call` data payload.
    public static func toolName(from data: [String: AnyCodableLike]?) -> String? {
        guard let data else { return nil }
        if let display = toolDisplayObject(from: data) {
            if let title = display["title"]?.asString, !title.isEmpty {
                // Prefer the short tool name when title is a long "name: detail".
                if let name = display["tool_name"]?.asString ?? data["name"]?.asString,
                   !name.isEmpty {
                    return name
                }
                return title
            }
            if let name = display["tool_name"]?.asString, !name.isEmpty { return name }
        }
        if let name = data["name"]?.asString, !name.isEmpty { return name }
        if let title = data["title"]?.asString, !title.isEmpty { return title }
        if let tool = data["tool"]?.asString, !tool.isEmpty { return tool }
        if let toolName = data["tool_name"]?.asString, !toolName.isEmpty { return toolName }
        return nil
    }

    /// Human-readable one-line detail for the notch process row.
    public static func toolDescription(from data: [String: AnyCodableLike]?) -> String? {
        guard let data else { return nil }
        if let display = toolDisplayObject(from: data) {
            if let title = display["title"]?.asString, !title.isEmpty {
                return truncate(title, max: 160)
            }
            if case .object(let fields) = display["primary_fields"] {
                let parts = fields.compactMap { key, value -> String? in
                    guard let text = value.asString, !text.isEmpty else { return nil }
                    return "\(key)=\(text)"
                }
                if !parts.isEmpty {
                    return truncate(parts.sorted().joined(separator: " "), max: 160)
                }
            }
        }
        if let command = data["input"]?.asObject?["command"]?.asString, !command.isEmpty {
            return truncate(command, max: 160)
        }
        if let query = data["input"]?.asObject?["query"]?.asString, !query.isEmpty {
            return truncate(query, max: 160)
        }
        if let path = data["input"]?.asObject?["path"]?.asString, !path.isEmpty {
            return truncate(path, max: 160)
        }
        return toolName(from: data).map { truncate($0, max: 80) }
    }

    public static func progressText(from data: [String: AnyCodableLike]?) -> String? {
        guard let data else { return nil }
        if let text = data["text"]?.asString, !text.isEmpty {
            return truncate(text, max: 160)
        }
        if let signal = data["readable_summary"]?.asObject?["signal"]?.asString, !signal.isEmpty {
            return truncate(signal, max: 160)
        }
        return nil
    }

    public static func progressLabel(from data: [String: AnyCodableLike]?, eventName: String) -> String {
        if let phase = data?["phase"]?.asString, !phase.isEmpty {
            return compactPhaseLabel(phase)
        }
        switch eventName {
        case "stream.thinking_delta": return "thinking"
        case "stream.text_delta": return "reply"
        case "stream.progress": return "progress"
        case "stream.fallback_started": return "fallback"
        case "stream.ack": return "ack"
        default: return "working"
        }
    }

    public static func compactPhaseLabel(_ phase: String) -> String {
        switch phase {
        case "context_build": return "context"
        case "llm_request": return "llm"
        case "llm_streaming": return "llm"
        case "tool_execution": return "tool"
        case "post_processing": return "post"
        case "provider_waiting": return "waiting"
        case "provider_retrying": return "retry"
        default:
            return truncate(phase.replacingOccurrences(of: "_", with: " "), max: 24)
        }
    }

    public static func truncate(_ text: String, max: Int) -> String {
        guard text.count > max else { return text }
        return String(text.prefix(max - 1)) + "…"
    }

    private static func toolDisplayObject(from data: [String: AnyCodableLike]) -> [String: AnyCodableLike]? {
        if let direct = data["agentix.tool_display.v1"]?.asObject { return direct }
        if let meta = data["_meta"]?.asObject {
            if let v = meta["agentix.tool_display.v1"]?.asObject { return v }
            if let agentix = meta["agentix"]?.asObject,
               let v = agentix["tool_display"]?.asObject ?? agentix["tool_display.v1"]?.asObject {
                return v
            }
        }
        return nil
    }

    /// Display label aligned with Hooks settings: `AiWork` / `AiWork CLI`.
    ///
    /// Prefer `client_type` from `sessions.get`. The daemon still emits
    /// `DTCoderGUI` / `DTCoderTUI`; accept `AiWork*` as well after the rename.
    /// Modern `aiwork`/`aiwork tui` attaches via ACP, so both GUI and TUI
    /// session ids often share the `acp:` prefix — prefix alone is not enough.
    public static func sourceLabel(
        forDaemonSessionId sessionId: String?,
        clientType: String? = nil
    ) -> String {
        if let label = label(forClientType: clientType) {
            return label
        }
        guard let sessionId else { return "AiWork" }
        if sessionId.hasPrefix("cli:") { return "AiWork CLI" }
        if sessionId.hasPrefix("acp:") { return "AiWork" }
        if sessionId.hasPrefix("gateway:") { return "AiWork" }
        return "AiWork"
    }

    public static func label(forClientType clientType: String?) -> String? {
        guard let raw = clientType?.trimmingCharacters(in: .whitespacesAndNewlines),
              !raw.isEmpty else { return nil }
        switch raw {
        case "DTCoderGUI", "DTCoderGui", "AiWorkGUI", "AiWorkGui":
            return "AiWork"
        case "DTCoderTUI", "DTCoderTui", "DTCoderCLI", "DTCoderCli",
             "AiWorkTUI", "AiWorkTui", "AiWorkCLI", "AiWorkCli":
            return "AiWork CLI"
        default:
            break
        }
        let lower = raw.lowercased()
        if lower.contains("tui") || lower.contains("cli") { return "AiWork CLI" }
        if lower.contains("gui") { return "AiWork" }
        return nil
    }

    /// CodeIsland `source` key: GUI → `aiwork`, TUI/CLI → `aiwork-cli`.
    /// GUI and CLI/TUI get separate keys so each surface toggles independently.
    public static func codeIslandSource(
        clientType: String?,
        daemonSessionId: String? = nil
    ) -> String {
        if let label = label(forClientType: clientType) {
            return label == "AiWork CLI" ? "aiwork-cli" : "aiwork"
        }
        // Legacy session-id prefixes when client_type is not hydrated yet.
        if let daemonSessionId, daemonSessionId.hasPrefix("cli:") {
            return "aiwork-cli"
        }
        return "aiwork"
    }

    /// Whether a `sessions.list` entry looks actively busy (for backfill).
    public static func isActiveListEntry(_ entry: [String: AnyCodableLike]) -> Bool {
        if let status = entry["status"]?.asString {
            switch status {
            case "waiting_approval",
                 "waiting_plan_confirmation",
                 "running":
                return true
            default:
                break
            }
        }
        if let phase = entry["readable_summary"]?.asObject?["phase"]?.asString {
            switch phase {
            case "idle", "none", "":
                break
            default:
                return true
            }
        }
        if entry["external_turn"]?.asObject != nil {
            return true
        }
        return false
    }

    public static func agentStatus(fromListEntry entry: [String: AnyCodableLike]) -> AgentStatus {
        if let status = entry["status"]?.asString {
            switch status {
            case "waiting_approval", "waiting_plan_confirmation":
                return .waitingApproval
            case "running":
                return .running
            default:
                break
            }
        }
        if let waitingFor = entry["external_turn"]?.asObject?["waiting_for"]?.asString,
           waitingFor == "question" {
            return .waitingQuestion
        }
        if let phase = entry["readable_summary"]?.asObject?["phase"]?.asString {
            switch phase {
            case "waiting_answer":
                return .waitingQuestion
            case "waiting_approval", "waiting_plan_confirmation":
                return .waitingApproval
            case "tool_execution":
                return .running
            case "idle":
                return .idle
            default:
                return .processing
            }
        }
        if entry["external_turn"]?.asObject != nil {
            return .processing
        }
        return .processing
    }

    /// Whether a locally non-idle AiWork session should be forced idle because
    /// the daemon no longer reports it in `active_request_details`.
    public static func shouldForceIdleSession(
        daemonSessionId: String,
        lastActivity: Date,
        busyDaemonIds: Set<String>,
        now: Date = Date(),
        grace: TimeInterval = 8
    ) -> Bool {
        if busyDaemonIds.contains(daemonSessionId) { return false }
        // Grace avoids racing a turn that just started before stats refresh.
        return now.timeIntervalSince(lastActivity) > grace
    }
}
