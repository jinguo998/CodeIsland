import Foundation
import CodeIslandCore

typealias IdleSessionRemovalSelection = CodeIslandCore.IdleSessionRemovalSelection
typealias IdleSessionRemovalPolicy = CodeIslandCore.IdleSessionRemovalPolicy

/// Passive snapshots may be stale and cannot undo an explicit removal. Runtime work and requests
/// are user-visible evidence that the conversation is active again and may restore its whole group.
enum RemovedSessionRevivalEvidence {
    case activeWork
    case blockingRequest
    case passive

    /// Hooks that start something are work: a turn, a tool, a subagent (Gemini's
    /// BeforeAgent included), a compaction. The rest trail a finished turn —
    /// Stop, the idle_prompt Notification a minute later, SessionEnd — or
    /// report on work an earlier hook started.
    static let workStartingHooks: Set<String> = [
        "SessionStart", "UserPromptSubmit", "PreToolUse", "SubagentStart", "PreCompact",
    ]

    /// What a hook that reaches `AppState.handleEvent` says about a removed card.
    /// A terminal permission prompt (`Notification(permission_prompt)`) blocks
    /// the agent on the user like a queued request does, so it always surfaces.
    static func forHookEvent(_ event: HookEvent, normalizedEventName: String) -> Self {
        if workStartingHooks.contains(normalizedEventName) { return .activeWork }
        if normalizedEventName == "Notification",
           QuestionPayload.from(event: event) != nil
            || AppState.displayOnlyWaitAsk(forHookEvent: event, normalizedEventName: normalizedEventName) != nil {
            return .blockingRequest
        }
        return .passive
    }
}
