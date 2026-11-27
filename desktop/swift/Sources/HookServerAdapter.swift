import CodeIslandCore
import Foundation

// os.Logger is unavailable on Windows. These three diagnostics contain no payload data.
struct DesktopHookLog {
    func warning(_ message: String) { FileHandle.standardError.write(Data((message + "\n").utf8)) }
    func error(_ message: String) { warning(message) }
}
let log = DesktopHookLog()

struct DesktopQuestion {
    let event: HookEvent
    let isFromPermission: Bool
}

struct DesktopWebhookFilter {
    let filter: String?
    func string(forKey key: String) -> String? { filter }
}
enum SettingsKey { static let webhookEventFilter = "webhookEventFilter" }

private func hookFields(_ event: HookEvent) -> [String: Any] {
    let route = HookServer.routeKind(for: event)
    let askTool = event.toolName == "AskUserQuestion" || event.toolName == "ask_user_question"
    let items = askTool ? DesktopHookAnswers.askItems(event) : []
    let notification = QuestionPayload.from(event: event)
    var fields: [String: Any] = ["route": route == .event ? "event" :
        (route == .question || askTool ? "question" : "permission")]
    fields["tool"] = event.toolName
    fields["agentId"] = event.agentId
    fields["session"] = SubsessionRouter.sessionId(in: event.rawJSON)
    if let parent = SubsessionRouter.parentCardId(in: event.rawJSON) {
        // The SSH transport applies its persisted Windows host namespace after projection.
        let prefix = HookEvent.cardSessionId(forRawSessionId: "", in: event.rawJSON)
        if SubsessionRouter.remoteHostId(in: event.rawJSON) != nil, parent.hasPrefix(prefix) {
            fields["parent"] = String(parent.dropFirst(prefix.count))
        } else {
            fields["parent"] = parent
        }
    }
    fields["transcript"] = SubsessionRouter.transcriptPath(in: event.rawJSON)
    fields["question"] = items.first?.payload.question ?? notification?.question
    fields["items"] = items.map { item -> [String: Any] in
        var row: [String: Any] = ["question": item.payload.question, "answer_key": item.answerKey,
            "multiSelect": item.multiSelect, "isSecret": item.payload.isSecret]
        row["header"] = item.payload.header
        row["options"] = item.payload.options?.enumerated().map { index, label -> [String: String] in
            var option = ["label": label]
            if let descriptions = item.payload.descriptions, descriptions.indices.contains(index) {
                option["description"] = descriptions[index]
            }
            return option
        }
        return row
    }
    fields["options"] = items.isEmpty ? notification?.options : items.flatMap { $0.payload.options ?? [] }
    return fields
}

func desktopHookOperation(_ operation: String, _ data: Data) throws -> Data {
    let args = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    let encoder = JSONEncoder()
    let empty = Data("null".utf8)
    switch operation {
    case "hook.source":
        return try encoder.encode(SessionSnapshot.normalizedSupportedSource(args["source"] as? String))
    case "hook.fields":
        guard let event = try bridgeHook(data) else { return empty }
        return try JSONSerialization.data(withJSONObject: hookFields(event))
    case "hook.localOrigin":
        return try encoder.encode(!HookOrigin.claimsRemoteHost(args["payload"] as? [String: Any] ?? [:]))
    case "hook.receive":
        let origin: HookOrigin
        if let host = args["host"] as? [String: String] {
            guard let id = host["id"], let name = host["name"] else { return empty }
            origin = .remoteHost(id: id, name: name)
        } else {
            origin = .local
        }
        guard let row = args["payload"] as? [String: Any] else { return empty }
        let wire = try JSONSerialization.data(withJSONObject: row, options: .withoutEscapingSlashes)
        guard case .route(let bytes, let edge) = HookServer.intake(wire, origin: origin),
              let event = HookEvent(from: bytes) else { return empty }
        let route = HookServer.routeKind(for: event)
        let result: [String: Any] = ["payload": event.rawJSON, "event": event.eventName, "fields": hookFields(event),
            "source": event.rawJSON["_source"] as? String ?? "claude",
            "deferToProvider": HookServer.shouldDeferPermissionRequestToProvider(event),
            "waits": (edge?.waitsForAnswer ?? true) && route != .event]
        return try JSONSerialization.data(withJSONObject: result)
    case "hook.remoteFilter":
        let filter = args["filter"] as? String ?? ""
        let allowed = HookServer.remoteEventPassesCwdFilter(cwd: args["cwd"] as? String,
            workspaceRoots: args["roots"] as? [String], filterCSV: filter)
        let session = args["session"] as? String
        let tracked = args["tracked"] as? Bool == true ? Set([session].compactMap { $0 }) : []
        return try encoder.encode(allowed || HookServer.remoteEventBypassesCwdFilter(
            eventName: args["event"] as? String ?? "", sessionId: session, trackedSessionIds: tracked))
    case "hook.policy":
        guard let event = try bridgeHook(data) else { return try encoder.encode("pass") }
        if let cwd = event.rawJSON["cwd"] as? String,
           HookServer.cwdMatchesAnyPattern(cwd, patternsCSV: args["excluded"] as? String ?? "") {
            return try encoder.encode("exclude")
        }
        guard HookServer.routeKind(for: event) == .permission, event.toolName != "AskUserQuestion" else {
            return try encoder.encode("pass")
        }
        let tools = Set((args["tools"] as? String ?? "").split(separator: ",").map(String.init))
        let custom = try JSONSerialization.data(withJSONObject: args["custom"] as? [[String: Any]] ?? [])
        return try DesktopSourcePreferences.$customCLIs.withValue(custom) {
            let sources = HookServer.approvedSources(args["approved"] as? String ?? "")
            let approved = event.toolName.map(tools.contains) == true || HookServer.isAutoApprovedSource(event, approved: sources)
            return try encoder.encode(approved ? "approve" : "pass")
        }
    case "hook.webhook":
        guard let event = try bridgeHook(data) else { return empty }
        var encoded = empty
        DesktopWebhook.encode(event, defaults: .init(filter: args["filter"] as? String)) { encoded = $0 }
        return encoded
    case "hook.keepsRule":
        guard let event = try bridgeHook(data) else { return try encoder.encode(false) }
        return try encoder.encode(DesktopHookAnswers.agentKeepsAlwaysRule(for: event))
    default:
        guard let event = try bridgeHook(data) else { return empty }
        return try encoder.encode(String(decoding: desktopAnswer(event, args: args), as: UTF8.self))
    }
}

private func desktopAnswer(_ event: HookEvent, args: [String: Any]) -> Data {
    let action = args["action"] as? String ?? "skip"
    guard ["approve", "always_allow", "answer", "deny", "skip"].contains(action) else { return Data() }
    let denied = action == "deny" || action == "skip"
    let format = args["format"] as? String
    let question = args["question"] as? Bool == true
    let notification = event.eventName.caseInsensitiveCompare("Notification") == .orderedSame
    let answer = args["text"] as? String ?? ""
    if format == "claude" && (question || notification) ||
        format == "notification" && (question || notification && action == "answer") {
        let isPermission = format == "claude" && (!notification || event.toolInput != nil)
        if denied { return DesktopHookAnswers.skip(isPermission: isPermission) }
        let key = DesktopHookAnswers.askItems(event).first?.answerKey ?? "answer"
        let choices = try? JSONDecoder().decode([String: String].self, from: Data(answer.utf8))
        return DesktopHookAnswers().question(.init(event: event, isFromPermission: isPermission), answer,
            answers: choices.flatMap { $0.isEmpty ? nil : $0 } ?? [key: answer])
    }
    if format == "gemini" {
        let transport = DesktopAnswerTransport(waitsForAnswer: true, answerFormat: .geminiDecision)
        return transport.stdout(forAnswer: denied ? DesktopHookAnswers.deny() : DesktopHookAnswers.plainAllowAnswer) ?? Data()
    }
    if format == "claude" || format == "codex" {
        if denied { return DesktopHookAnswers.deny() }
        if format == "claude", action == "always_allow", DesktopHookAnswers.agentKeepsAlwaysRule(for: event) {
            return DesktopHookAnswers.claudeAlwaysAllowResponse(toolName: event.toolName ?? "",
                bareToolName: SessionSnapshot.normalizedSupportedSource(event.rawJSON["_source"] as? String) == "minimax")
        }
        return DesktopHookAnswers.plainAllowAnswer
    }
    // Plain JSON clients predate the named pipe's native hook frames.
    return Data((denied ? #"{"decision":"deny"}"# : #"{"decision":"allow"}"#).utf8)
}
