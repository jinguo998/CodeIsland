import CodeIslandCore
import Foundation

// Expose the desktop snapshot through the fields the original companion
// publisher reads. No SwiftUI, Bluetooth delegate or AppState singleton is needed.
struct DesktopCompanion {
    struct DisplayEvent { let sessionId: String?; let toolName: String?; let toolDescription: String? }
    struct Group { let items: [AskUserQuestionItem]; let answers: [String: String] = [:] }
    struct Request { let event: DisplayEvent; let question: QuestionPayload; let askUserQuestionState: Group? }
    struct SettingsManager {
        let defaultSource: String
        @TaskLocal static var shared = Self(defaultSource: "claude")
    }
    static let secretQuestionPlaceholder = "Sensitive prompt — answer on this computer"
    let rotatingSessionId: String? = nil
    var sessions: [String: SessionSnapshot] = [:]
    var permissionQueue: [Request] = []
    var questionQueue: [Request] = []
    var activeSessionId: String?
    var primarySource: String
    var pendingPermission: Request? { permissionQueue.first }
    var pendingQuestion: Request? { questionQueue.first }

    // The original formatter caches are confined to the main thread on macOS.
    // The desktop can publish from more than one transport thread.
    private static let formatLock = NSLock()

    static func state(_ data: Data) throws -> Data {
        struct Message: Decodable { let user: Bool; let text: String }
        struct Card: Decodable {
            let id: String, source: String, status: String, executing: Bool, activity: Double
            let cwd: String?, tool: String?, detail: String?, prompt: String?, reply: String?
            let messages: [Message]
        }
        struct Item: Decodable {
            let question: String, header: String?, options: [String], optionDescriptions: [String]?
            let isSecret: Bool?, multiSelect: Bool?, answerKey: String?
            var native: AskUserQuestionItem {
                let secret = isSecret ?? false
                let payload = QuestionPayload(question: secret ? DesktopCompanion.secretQuestionPlaceholder : question,
                    options: secret ? [] : options, descriptions: secret ? [] : optionDescriptions,
                    header: header, isSecret: secret)
                return .init(payload: payload, answerKey: answerKey ?? header ?? question, multiSelect: multiSelect ?? false)
            }
        }
        struct WireRequest: Decodable {
            let sessionId: String, kind: String, tool: String?, body: String?, title: String?
            let options: [String], questions: [Item]?, hidden: Bool
        }
        struct Input: Decodable {
            let cards: [Card], requests: [WireRequest], current: String?, source: String, sequence: UInt64, now: Double
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let input = try decoder.decode(Input.self, from: data)
        var model = Self(activeSessionId: input.current, primarySource: input.source)
        for card in input.cards {
            var session = SessionSnapshot()
            session.source = card.source
            session.status = nativeStatus(card.status, executing: card.executing)
            session.lastActivity = Date(timeIntervalSince1970: card.activity)
            session.cwd = card.cwd.map(DesktopPath.withForwardSlashes)
            session.currentTool = card.tool
            session.toolDescription = card.detail
            session.lastUserPrompt = card.prompt
            session.lastAssistantMessage = card.reply
            session.recentMessages = card.messages.map { ChatMessage(isUser: $0.user, text: $0.text) }
            model.sessions[card.id] = session
        }
        for row in input.requests where !row.hidden {
            let items = (row.questions ?? []).map(\.native)
            let question = items.first?.payload ?? QuestionPayload(question: row.body ?? row.title ?? "", options: row.options)
            let event = DisplayEvent(sessionId: row.sessionId, toolName: row.tool, toolDescription: row.body ?? row.title)
            let request = Request(event: event, question: question, askUserQuestionState: items.isEmpty ? nil : Group(items: items))
            if row.kind == "permission" { model.permissionQueue.append(request) } else { model.questionQueue.append(request) }
        }
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        formatLock.lock()
        let payload = SettingsManager.$shared.withValue(.init(defaultSource: input.source)) {
            model.appleCompanionStatePayload(sequence: input.sequence)
        }
        formatLock.unlock()
        // Preserve the timestamp supplied by the transport and deterministic replay.
        var document = try JSONSerialization.jsonObject(with: encoder.encode(payload)) as? [String: Any] ?? [:]
        document["updatedAt"] = ISO8601DateFormatter().string(from: Date(timeIntervalSince1970: input.now))
        return try JSONSerialization.data(withJSONObject: document)
    }
}
