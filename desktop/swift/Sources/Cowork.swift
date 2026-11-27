import CodeIslandCore
import Foundation

private struct Metadata: Encodable {
    let id: String
    let cli_id: String?
    let title: String?
    let initial_prompt: String?
    let host_cwd: String?
    let model: String?
    let created_at: Int64?
    let last_activity: Int64?
    let archived: Bool
    let session_type: String?

    init(_ value: CoworkSessionMetadata) {
        id = value.sessionId
        cli_id = value.cliSessionId
        title = value.displayTitle
        initial_prompt = value.initialMessage
        host_cwd = value.hostCwd
        model = value.model
        created_at = value.createdAt.map(microseconds)
        last_activity = value.lastActivityAt.map(microseconds)
        archived = value.isArchived
        session_type = value.sessionType
    }
}

func coworkMetadata(_ bytes: Data) throws -> Data {
    try JSONEncoder().encode(CoworkSessionMetadata.parse(bytes).map(Metadata.init))
}

private struct Tool: Encodable { let name: String; let detail: String? }

private struct Audit: Encodable {
    let phase: String
    let prompt_count: Int
    let end_count: Int
    let request_count: Int
    let prompt: String?
    let result: String?
    let failed: Bool
    let interrupted: Bool
    let current_tool: Tool?
    let permission: Tool?

    init(_ value: CoworkAuditState) {
        switch value.phase {
        case .idle: phase = "idle"
        case .processing: phase = "running"
        case .waitingApproval: phase = "waiting_approval"
        case .waitingQuestion: phase = "waiting_question"
        }
        prompt_count = value.promptCount
        end_count = value.completedTurnCount
        request_count = value.permissionRequestCount
        prompt = value.lastPrompt
        result = value.lastResultText
        failed = value.lastTurnFailed
        interrupted = value.lastTurnInterrupted
        current_tool = value.currentTool.map { Tool(name: $0.name, detail: $0.detail) }
        permission = value.activePermission.map { Tool(name: $0.toolName, detail: $0.detail) }
    }
}

func coworkBatch(_ bytes: Data, state: inout CoworkAuditState) throws -> Data {
    struct Batch: Encodable {
        let live: Bool
        let prompts: Bool
        let turns: Bool
        let permissions: Bool
        let audit: Audit
    }
    let events = CoworkAuditParser.events(in: bytes).events
    let before = state
    state.apply(events)
    return try JSONEncoder().encode(Batch(live: events.contains { $0 != .ignored },
        prompts: state.promptCount != before.promptCount, turns: state.completedTurnCount != before.completedTurnCount,
        permissions: state.permissionRequestCount != before.permissionRequestCount, audit: Audit(state)))
}
