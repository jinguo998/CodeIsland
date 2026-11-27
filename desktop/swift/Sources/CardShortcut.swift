import Foundation

/// The host applies the returned action under its store lock and correlates the
/// named-pipe reply with the request ID. The native router sees each visible card.
final class DesktopCardShortcut {
    struct Event { let sessionId: String? }
    struct Request { let event: Event }
    let permissionQueue: [Request], questions: [Request], surface: IslandSurface
    var visiblePermissionSessionIds: Set<String> { Set(permissionQueue.compactMap(\.event.sessionId)) }
    var nextVisibleQuestion: Request? { questions.first }
    func pendingQuestion(forSession session: String) -> Request? { questions.first { $0.event.sessionId == session } }

    struct Effect: Encodable { let action: String; let session: String? }
    private var effect = Effect(action: "ignored", session: nil)
    func approvePermission(always: Bool = false, expectedSessionId: String) {
        effect = Effect(action: always ? "always_allow" : "approve", session: expectedSessionId)
    }
    func denyPermission(expectedSessionId: String) { effect = Effect(action: "deny", session: expectedSessionId) }
    func skipQuestion(expectedSessionId: String) { effect = Effect(action: "skip", session: expectedSessionId) }
    func openPendingApprovalCard(sessionId: String) { effect = Effect(action: "open", session: sessionId) }
    func openPendingQuestionCard(sessionId: String) { effect = Effect(action: "open", session: sessionId) }
    func foldDeadCard() { effect = Effect(action: "ignored", session: nil) }

    init(permissions: [String], questions: [String], surface: IslandSurface) {
        self.surface = surface
        permissionQueue = permissions.map { Request(event: .init(sessionId: $0)) }
        self.questions = questions.map { Request(event: .init(sessionId: $0)) }
    }

    static func route(_ data: Data) throws -> Data {
        struct Input: Decodable {
            let action: String, permissions: [String], questions: [String], rendered: Bool
            let current: String?, kind: String?
        }
        let args = try JSONDecoder().decode(Input.self, from: data)
        let surface: IslandSurface = args.current.map {
            args.kind == "permission" ? .approvalCard(sessionId: $0) : .questionCard(sessionId: $0)
        } ?? .collapsed
        let router = Self(permissions: args.permissions, questions: args.questions, surface: surface)
        let action = ShortcutAction(rawValue: args.action) ?? .togglePanel
        if case .opened(let card) = router.performCardShortcut(action, islandOnScreen: args.rendered) {
            return try JSONEncoder().encode(Effect(action: "open", session: card.sessionId))
        }
        return try JSONEncoder().encode(router.effect)
    }
}
