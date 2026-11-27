import CodeIslandCore
import Foundation

/// A card restored from the desktop store can predate the Swift core. The
/// Windows host also changes fields when a request is answered or a WSL
/// process exits. Import its visible values before reducing the next event.
private struct HostCard: Decodable {
    let status: String
    let executing: Bool
    let tool: String?
    let detail: String?
    let id: String?
    let prompt: String?
    let reply: String?
    let live: String?
    let interrupted: Bool
    let failed: Bool
    let tasks: [TaskRow]
}

struct TaskRow: Codable, Equatable {
    let id: String
    let title: String
    let status: AgentTaskStatus

    init(_ item: AgentTaskItem) {
        id = item.taskId ?? item.id
        title = item.title
        status = item.status
    }

    var restored: AgentTaskItem { .init(id: id, taskId: id, title: title, status: status) }
}

private extension HostCard {
    func synchronize(_ session: inout SessionSnapshot) {
        session.status = nativeStatus(status, executing: executing)
        session.currentTool = tool
        session.currentToolUseId = id
        session.toolDescription = detail
        session.lastUserPrompt = prompt
        session.lastAssistantMessage = reply
        session.liveCodexOutput = live
        session.interrupted = interrupted
        session.lastTurnFailed = failed
        if session.agentTasks.items.map(TaskRow.init) != tasks {
            session.agentTasks = AgentTaskList(items: tasks.map(\.restored))
        }
    }
}

private struct ReductionArguments: Decodable {
    let previous: HostCard
    let card: String
    let fresh: Bool
    let limit: Int
}

private struct ReductionResult: Encodable {
    let removed: Bool
    let applied: Bool
    let ended: Bool
    let status: String
    let tool: String?
    let id: String?
    let detail: String?
    let executing: Bool
    let prompt: String?
    let reply: String?
    let live: String?
    let interrupted: Bool
    let failed: Bool
    let tasks: [TaskRow]
}

struct DesktopSessions {
    var cards: [String: SessionSnapshot] = [:]

    mutating func apply(_ data: Data) throws -> Data {
        let args = try JSONDecoder().decode(ReductionArguments.self, from: data)
        guard let event = try bridgeHook(data) else { return Data("null".utf8) }
        switch EventNormalizer.normalize(event.eventName) {
        case "SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure",
             "PermissionDenied", "AfterAgentResponse", "AgentTurnSettled", "TaskRoundComplete",
             "Stop", "Interrupt", "Notification", "PreCompact", "PostCompact":
            break
        default:
            // Synthetic events from the desktop process and transcript watchers
            // have their own transport contract.
            return Data("null".utf8)
        }
        if !args.fresh {
            var previous = cards[args.card] ?? SessionSnapshot()
            args.previous.synchronize(&previous)
            cards[args.card] = previous
        }
        let reduction = reduceHookEvent(sessions: &cards, event: event, maxHistory: args.limit)
        let removed = reduction.effects.contains(.removeSession(sessionId: args.card))
        let session = cards[args.card] ?? SessionSnapshot()
        let facts = reduction.facts
        // A failed turn is an idle Swift card with an error outcome. The
        // desktop view model exposes those as separate status values.
        let failed = facts.endedTurn && !session.interrupted && facts.turnError != nil
        let status: String
        switch session.status {
        case .idle where session.interrupted: status = "cancelled"
        case .idle where failed || session.lastTurnFailed: status = "failed"
        case .idle where facts.endedTurn: status = "completed"
        case .idle: status = "idle"
        case .running, .processing: status = "running"
        case .waitingApproval: status = "waiting_permission"
        case .waitingQuestion: status = "waiting_answer"
        }
        if removed { cards.removeValue(forKey: args.card) }
        return try JSONEncoder().encode(ReductionResult(removed: removed, applied: facts.appliedToSession,
            ended: facts.endedTurn, status: status, tool: session.currentTool, id: session.currentToolUseId,
            detail: session.toolDescription, executing: session.status == .running && session.currentTool != nil,
            prompt: session.lastUserPrompt, reply: session.lastAssistantMessage, live: session.liveCodexOutput,
            interrupted: session.interrupted, failed: failed || session.lastTurnFailed,
            tasks: session.agentTasks.items.map(TaskRow.init)))
    }
}

private struct TranscriptKind: Encodable {
    let recap: String?
    let checklist: Bool
}

func classifyTranscript(_ data: Data) throws -> Data {
    let row = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    let recap = SessionRecap.from(transcriptLine: row)
    let events = AgentTaskTranscript.events(fromLine: row)
    return try JSONEncoder().encode(TranscriptKind(recap: recap?.text, checklist: !events.isEmpty))
}

extension DesktopSessions {
    mutating func applyChecklist(_ data: Data) throws -> Data {
        struct Arguments: Decodable { let card: String; let tasks: [TaskRow]; let now: Int64; let shared: Bool? }
        let args = try JSONDecoder().decode(Arguments.self, from: data)
        let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        let events: [AgentTaskEvent]
        if let row = json?["row"] as? [String: Any] {
            events = AgentTaskTranscript.events(fromLine: row)
        } else if let event = try bridgeHook(data) {
            events = AgentTaskHookParser.events(from: event, normalizedEventName: EventNormalizer.normalize(event.eventName))
        } else {
            return Data("null".utf8)
        }
        guard !events.isEmpty else { return Data("null".utf8) }
        var session = cards[args.card] ?? SessionSnapshot()
        if session.agentTasks.items.map(TaskRow.init) != args.tasks {
            session.agentTasks = AgentTaskList(items: args.tasks.map(\.restored))
        }
        let now = Date(timeIntervalSince1970: Double(args.now) / 1_000_000)
        if args.shared == true {
            session.agentTasks.applySharedUpdates(events, now: now)
        } else {
            session.agentTasks.apply(events, now: now)
        }
        cards[args.card] = session
        return try JSONEncoder().encode(session.agentTasks.items.map(TaskRow.init))
    }
}
