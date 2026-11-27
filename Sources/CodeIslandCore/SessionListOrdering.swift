import Foundation

// MARK: - Status word

/// The status a session card spells out in its chip and colours its edge
/// rail with. The word carries the meaning; the hue only repeats it.
public enum SessionCardStatus: Equatable, CaseIterable {
    /// An approval or a question is waiting on the user.
    case needsYou
    /// A tool is running.
    case working
    /// The model is thinking (no tool yet).
    case thinking
    /// Idle after a finished turn.
    case done
    /// Idle and nothing has happened yet.
    case idle
    /// The user interrupted the last turn.
    case stopped
    /// The last turn ended on an error (rate limit, overload, a failed stream).
    case error

    public init(_ session: SessionSnapshot) {
        switch session.status {
        case .waitingApproval, .waitingQuestion:
            self = .needsYou
        case .running:
            self = .working
        case .processing:
            self = .thinking
        case .idle:
            if session.lastTurnFailed {
                self = .error
            } else if session.interrupted {
                self = .stopped
            } else if session.lastAssistantMessage != nil
                        || session.lastUserPrompt != nil
                        || !session.recentMessages.isEmpty {
                self = .done
            } else {
                self = .idle
            }
        }
    }

    public var labelKey: String {
        switch self {
        case .needsYou: return "session_status_needs_you"
        case .working: return "session_status_working"
        case .thinking: return "session_status_thinking"
        case .done: return "session_status_done"
        case .idle: return "session_status_idle"
        case .stopped: return "session_status_stopped"
        case .error: return "session_status_error"
        }
    }

    /// Sort tier: what needs you, then what is working, then the rest.
    public var tier: Int {
        switch self {
        case .needsYou: return 0
        case .working, .thinking: return 1
        case .done, .idle, .stopped, .error: return 2
        }
    }
}

// MARK: - Ordering

/// The expanded list's order: sessions that need you first (in the order
/// their requests queued, so the one the buttons act on leads), then the
/// ones working (newest session first — a working session's activity changes
/// every few seconds, and sorting on it would shuffle the cards constantly),
/// then the rest by most recent activity.
public enum SessionListOrdering {
    public static func order(
        _ sessions: [String: SessionSnapshot],
        requestRank: [String: Int] = [:]
    ) -> [String] {
        let tiers = sessions.mapValues { SessionCardStatus($0).tier }
        return sessions.keys.sorted { a, b in
            guard let sa = sessions[a], let sb = sessions[b] else { return a < b }
            let ta = tiers[a] ?? 2, tb = tiers[b] ?? 2
            if ta != tb { return ta < tb }
            switch ta {
            case 0:
                let ra = requestRank[a] ?? Int.max, rb = requestRank[b] ?? Int.max
                if ra != rb { return ra < rb }
                if sa.lastActivity != sb.lastActivity { return sa.lastActivity > sb.lastActivity }
            case 1:
                if sa.startTime != sb.startTime { return sa.startTime > sb.startTime }
            default:
                if sa.lastActivity != sb.lastActivity { return sa.lastActivity > sb.lastActivity }
            }
            return a < b
        }
    }

    /// Request queue position per session: approvals (they hold a tool call)
    /// before questions, each in arrival order.
    public static func requestRank(permissionSessionIds: [String], questionSessionIds: [String]) -> [String: Int] {
        var rank: [String: Int] = [:]
        for (index, id) in (permissionSessionIds + questionSessionIds).enumerated() where rank[id] == nil {
            rank[id] = index
        }
        return rank
    }
}
