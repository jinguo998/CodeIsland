import AppKit
import SwiftUI
import CodeIslandCore

// MARK: - Density

/// How much of each session the expanded list shows (Settings → Appearance).
enum SessionListDensity: String, CaseIterable {
    /// One card per session: status chip, prompt, reply, running tool.
    case comfortable
    /// One ~34pt row per session; only a session that needs you opens up to
    /// show its request and the inline actions.
    case compact

    /// Unknown or missing values read as the default.
    init(storedValue: String?) {
        self = storedValue.flatMap(SessionListDensity.init(rawValue:)) ?? .comfortable
    }

    var titleKey: String {
        switch self {
        case .comfortable: return "density_comfortable"
        case .compact: return "density_compact"
        }
    }
}

typealias SessionCardStatus = CodeIslandCore.SessionCardStatus
typealias SessionListOrdering = CodeIslandCore.SessionListOrdering

extension SessionCardStatus {
    var label: String { L10n.shared[labelKey] }

    /// Chip text and edge-rail colour. Every one reads at ≥ 4.5:1 on the
    /// card's near-black fill.
    var color: Color {
        switch self {
        case .needsYou: return Color(red: 1.0, green: 0.6, blue: 0.2)
        case .working, .thinking: return Color(red: 0.3, green: 0.85, blue: 0.4)
        case .done: return .white.opacity(0.62)
        case .idle: return .white.opacity(0.55)
        case .stopped: return Color(red: 1.0, green: 0.55, blue: 0.4)
        case .error: return Color(red: 1.0, green: 0.38, blue: 0.42)
        }
    }

    /// The rail is quieter than the chip for the resting states, so a list of
    /// finished sessions doesn't read as a column of alerts.
    var railColor: Color {
        switch self {
        case .done: return .white.opacity(0.28)
        case .idle: return .white.opacity(0.16)
        default: return color
        }
    }
}

/// "Group by status" sections.
enum SessionListGrouping {
    /// Section order: what waits on you, then what is running.
    static let statusSections: [(statuses: Set<AgentStatus>, labelKey: String)] = [
        ([.waitingApproval, .waitingQuestion], "status_waiting"),
        ([.running], "status_running"),
        ([.processing], "status_processing"),
        ([.idle], "status_idle"),
    ]

    /// Non-empty sections, each keeping `orderedIds`' order. `status` says
    /// which section a session sits in — the held one while the pointer is on
    /// the list (SessionOrderFreeze.status), so a card doesn't change section
    /// under the cursor either; its chip still shows the live status.
    static func byStatus(
        _ orderedIds: [String],
        status: (String) -> AgentStatus?
    ) -> [(labelKey: String, ids: [String])] {
        statusSections.compactMap { section in
            let ids = orderedIds.filter { id in
                status(id).map { section.statuses.contains($0) } ?? false
            }
            return ids.isEmpty ? nil : (section.labelKey, ids)
        }
    }

    static func byStatus(
        _ orderedIds: [String],
        sessions: [String: SessionSnapshot]
    ) -> [(labelKey: String, ids: [String])] {
        byStatus(orderedIds) { sessions[$0]?.status }
    }

    /// "Group by CLI" sections, in display order. Every supported source has
    /// one (`cursor-cli` and `qoder-cli` sit with their IDE); a source missing
    /// here lands under "Other".
    static let cliSections: [(source: String, name: String)] = [
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("gemini", "Gemini"),
        ("antigravity", "AntiGravity"),
        ("google-antigravity", "Google Antigravity"),
        ("cursor", "Cursor"),
        ("trae", "Trae"),
        ("traecn", "Trae CN"),
        ("traecli", "Trae CLI"),
        ("copilot", "Copilot"),
        ("qoder", "Qoder"),
        ("qoderwork", "QoderWork"),
        ("droid", "Factory"),
        ("codebuddy", "CodeBuddy"),
        ("codybuddycn", "CodyBuddyCN"),
        ("stepfun", "StepFun"),
        ("workbuddy", "WorkBuddy"),
        ("hermes", "Hermes"),
        ("openclaw", "OpenClaw"),
        ("grok", "Grok CLI"),
        ("qwen", "Qwen Code"),
        ("kimi", "Kimi Code CLI"),
        ("opencode", "OpenCode"),
        ("mimo", "MiMo"),
        ("pi", "Pi"),
        ("kiro", "Kiro"),
        ("cline", "Cline"),
        ("zcode", "ZCode"),
        ("minimax", "MiniMax Code CLI"),
        ("dsh", "DeepSeek Harness"),
        ("aiwork", "AiWork"),
        ("aiwork-cli", "AiWork CLI"),
    ]
}

/// Holds the list's order while the pointer is over it, so a card never
/// moves out from under the cursor when a session changes status; the list
/// re-sorts when the pointer leaves.
struct SessionOrderFreeze: Equatable {
    private(set) var frozen: [String]?
    /// Statuses when the hold began: "Group by status" keeps each card in the
    /// section it was in.
    private(set) var frozenStatuses: [String: AgentStatus] = [:]

    /// How long the pointer may be off the list before it re-sorts — enough
    /// to cross the panel's edge and come back.
    static let releaseDelay: UInt64 = 250_000_000

    var isFrozen: Bool { frozen != nil }

    /// Pointer entered: keep what is on screen now. A second call while
    /// already frozen keeps the first order.
    mutating func freeze(_ current: [String], statuses: [String: AgentStatus] = [:]) {
        guard frozen == nil else { return }
        frozen = current
        frozenStatuses = statuses
    }

    /// Pointer left: back to the live order.
    mutating func thaw() {
        frozen = nil
        frozenStatuses = [:]
    }

    /// The status that places a session in a "Group by status" section: the
    /// held one while frozen (a session that started since uses its own).
    func status(of id: String, live: AgentStatus?) -> AgentStatus? {
        guard isFrozen, let held = frozenStatuses[id] else { return live }
        return held
    }

    /// The order to show. While frozen, sessions keep their places; a session
    /// that ended drops out and one that started joins at the end, so nothing
    /// already on screen moves.
    func apply(_ live: [String]) -> [String] {
        guard let frozen else { return live }
        let liveSet = Set(live)
        let kept = frozen.filter { liveSet.contains($0) }
        let keptSet = Set(kept)
        return kept + live.filter { !keptSet.contains($0) }
    }
}

extension AppState {
    /// Session ids in the expanded list's live (unfrozen) order.
    func sessionListOrder() -> [String] {
        let rank = SessionListOrdering.requestRank(
            permissionSessionIds: permissionQueue.map { $0.event.sessionId ?? "default" },
            questionSessionIds: questionQueue.map { $0.event.sessionId ?? "default" }
        )
        return SessionListOrdering.order(sessions, requestRank: rank)
    }
}

// MARK: - Compact metrics

enum CompactSessionRowMetrics {
    /// Height of one collapsed row.
    static let rowHeight: CGFloat = 34
    /// Gap between rows.
    static let spacing: CGFloat = 3

    /// Longest reply excerpt a row's tooltip carries; the completion card is
    /// where a whole reply is read.
    static let tooltipCharacters = 280

    /// "<name>\n<latest reply, cut to tooltipCharacters>".
    static func tooltip(name: String, latest: String?) -> String {
        guard let latest = latest?.trimmingCharacters(in: .whitespacesAndNewlines), !latest.isEmpty else {
            return name
        }
        let excerpt = latest.count > tooltipCharacters
            ? latest.prefix(tooltipCharacters).trimmingCharacters(in: .whitespacesAndNewlines) + "…"
            : latest
        return "\(name)\n\(excerpt)"
    }

    /// The status word, a little smaller than the row's text.
    static func statusFontSize(_ contentFontSize: CGFloat) -> CGFloat {
        max(8.5, contentFontSize - 2)
    }

    /// Width of the status-word column: the widest word in the current
    /// language, so the words line up in a column whatever the language.
    static func statusColumnWidth(fontSize: CGFloat) -> CGFloat {
        let font = NSFont.monospacedSystemFont(ofSize: fontSize, weight: .bold)
        let widest = SessionCardStatus.allCases
            .map { ($0.label as NSString).size(withAttributes: [.font: font]).width }
            .max() ?? 0
        return (widest + 2).rounded(.up)
    }

    /// Most the project column takes: ~16 characters at the default size,
    /// growing a little with the text so large sizes still leave the
    /// activity most of the row.
    static func projectColumnCap(fontSize: CGFloat) -> CGFloat {
        min(160 * fontSize / 11, 180)
    }

    /// Width of the project column: the widest name on screen, capped so the
    /// activity keeps most of the row.
    static func projectColumnWidth(names: [String], fontSize: CGFloat, cap: CGFloat) -> CGFloat {
        let font = NSFont.monospacedSystemFont(ofSize: fontSize, weight: .bold)
        let widest = names
            .map { ($0 as NSString).size(withAttributes: [.font: font]).width }
            .max() ?? 0
        return min(max(48, (widest + 2).rounded(.up)), cap)
    }
}
