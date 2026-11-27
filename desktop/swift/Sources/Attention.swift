import CodeIslandCore
import Foundation

private struct DesktopAttentionContext: Decodable {
    let soundsEnabled: Bool
    let audibleCues: Set<String>
    let quietHours: Bool
    let muteWhenAway: Bool
    let away: Bool
    let completionStyle: String
    let pushEnabled: Bool
    let sessionInFront: Bool

    var hold: SoundHold {
        SoundHold(quietHours: { quietHours }, muteWhenAway: muteWhenAway, sceneQuiet: { away })
    }

    var native: AttentionContext {
        AttentionContext(sounds: .init(enabled: soundsEnabled, audibleCues: audibleCues),
                         soundHold: hold, completionStyle: CompletionStyle(rawValue: completionStyle) ?? .expand,
                         pushEnabled: pushEnabled, sessionInFront: { sessionInFront })
    }
}

private struct EventArguments: Decodable {
    let moment: String
    let applied: Bool
    let ended: Bool
    let first: Bool
    let child: Bool
    let interrupted: Bool
    let agentId: String?
    let context: DesktopAttentionContext?
}

func attentionEvent(_ data: Data, settle: Bool) throws -> Data {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    let args = try decoder.decode(EventArguments.self, from: data)
    let subject = AttentionSubject(isChild: args.child, interrupted: args.interrupted)
    let moment: AttentionMoment?
    switch args.moment {
    case "silent": moment = nil
    case "approval": moment = .approvalQueued(firstOfBurst: args.first)
    case "question", "appServerQuestion":
        moment = .questionQueued(firstOfBurst: args.first, smartSuppressible: args.moment != "appServerQuestion")
    case "stopped": moment = .turnEnded(.stopped, host: .claudeDesktop)
    case "interrupted": moment = .turnEnded(.stopped, host: .aiwork)
    case "finished": moment = .turnEnded(.completed, host: .claudeDesktop)
    case "failed": moment = .turnEnded(.failed, host: .claudeDesktop)
    default:
        if let event = try bridgeHook(data) {
            var facts = HookEventFacts(event: event)
            facts.appliedToSession = args.applied
            facts.endedTurn = args.ended
            facts.subagentTurn = PushEventClassifier.isSubagentCompletion(
                agentId: args.agentId, raw: event.rawJSON, sessionId: event.sessionId ?? "", session: nil
            )
            moment = .hookEvent(facts)
        } else {
            moment = nil
        }
    }
    let intents = moment.map { AttentionPolicy.intents(for: $0, subject: subject) } ?? AttentionIntents()
    if settle, let context = args.context { return try encode(AttentionPolicy.settle(intents, context: context.native)) }
    return try encode(intents)
}

// Translate the Tauri event contract at the edge. The native policy decides
// whether to play, present, push or schedule a reminder.
private typealias Object = [String: Any]

private func pushName(_ kind: AttentionPushKind) -> String? {
    switch kind {
    case .approval: return "approval"
    case .question: return "question"
    case .displayOnlyWait: return "display_only_wait"
    case .error: return "error"
    case .completion(let child, let interrupted):
        let input = PushGateInput(kind: .completion, onlyWhenAway: false, idleThreshold: 1,
                                  presence: .init(), isSubagent: child, interrupted: interrupted)
        return PushGate.evaluate(input) == nil ? "completion" : nil
    }
}

private func nativePush(_ name: String?) -> AttentionPushKind? {
    switch name {
    case "approval": return .approval
    case "question": return .question
    case "display_only_wait": return .displayOnlyWait
    case "error": return .error
    case "completion": return .completion(isSubagent: false, interrupted: false)
    default: return nil
    }
}

private func followUp(_ intent: FollowUpIntent?) -> Any? {
    switch intent {
    case .trackCompletion(let failed): return ["track_completion": ["turn_failed": failed]]
    case .restartWait: return "restart_wait"
    case nil: return nil
    }
}

private func encode(_ value: AttentionIntents) throws -> Data {
    var result: Object = [:]
    result["sound"] = value.sound
    result["completion"] = value.completion.map { ["turn_failed": $0.turnFailed] }
    if let push = value.push, let name = pushName(push.kind) {
        result["push"] = ["kind": name, "smart_suppressible": push.smartSuppressible] as Object
    }
    result["follow_up"] = followUp(value.followUp)
    return try JSONSerialization.data(withJSONObject: result)
}

private func encode(_ value: AttentionDecision) throws -> Data {
    var result: Object = [:]
    result["sound"] = value.sound
    if let completion = value.completion {
        let presentation: String
        switch completion.presentation {
        case .statusOnly: presentation = "status_only"
        case .glance: presentation = "glance"
        case .card: presentation = "card"
        }
        result["completion"] = ["turn_failed": completion.turnFailed, "presentation": presentation] as Object
    }
    if let push = value.push, let name = pushName(push.kind) {
        result["push"] = ["kind": name, "smart_suppressed": push.smartSuppressed] as Object
    }
    result["follow_up"] = followUp(value.followUp)
    return try JSONSerialization.data(withJSONObject: result)
}

func attentionHold(_ data: Data) throws -> Data {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    let context = try decoder.decode(DesktopAttentionContext.self, from: data)
    return try JSONEncoder().encode(AttentionPolicy.holdsSounds(context.hold))
}

func settleAttention(_ data: Data) throws -> Data {
    struct Arguments: Decodable { let context: DesktopAttentionContext }
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    let context = try decoder.decode(Arguments.self, from: data).context
    let object = try JSONSerialization.jsonObject(with: data) as? Object
    let input = object?["intents"] as? Object ?? [:]
    var intents = AttentionIntents(sound: input["sound"] as? String)
    if let completion = input["completion"] as? Object, let failed = completion["turn_failed"] as? Bool {
        intents.completion = .init(turnFailed: failed)
    }
    if let push = input["push"] as? Object, let kind = nativePush(push["kind"] as? String) {
        intents.push = .init(kind: kind, smartSuppressible: push["smart_suppressible"] as? Bool ?? true)
    }
    if let follow = input["follow_up"] as? Object, let completion = follow["track_completion"] as? Object {
        intents.followUp = .trackCompletion(turnFailed: completion["turn_failed"] as? Bool ?? false)
    } else if input["follow_up"] as? String == "restart_wait" {
        intents.followUp = .restartWait(.approval)
    }
    return try encode(AttentionPolicy.settle(intents, context: context.native))
}
