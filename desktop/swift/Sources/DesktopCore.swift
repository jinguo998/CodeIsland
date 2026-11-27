import CodeIslandCore
import Foundation

// The caller owns each context and serializes access to it. Output bytes are
// borrowed only during the callback, so neither runtime frees the other's heap.
private final class DesktopContext {
    var reminders = FollowUpReminderScheduler()
    var quota = ClaudeQuotaScheduler()
    var usage = ClaudeUsageScanner.FileCache()
    var sessions = DesktopSessions()
    var cowork = CoworkAuditState()
    var cursor = CursorHookDeliveryFilter()
    var reconnect = RemoteReconnectPolicy()
    var pushPolicy = DesktopPushPolicy()
    var failureSounds = SoundDebouncer(window: EventSoundRouting.turnFailureWindow)

    func copy() -> DesktopContext {
        let result = DesktopContext()
        result.reminders = reminders
        result.quota = quota
        result.usage = usage
        result.sessions = sessions
        result.cowork = cowork
        result.cursor = cursor
        result.reconnect = reconnect
        result.pushPolicy = pushPolicy
        result.failureSounds = failureSounds
        return result
    }
}

@_cdecl("codeisland_core_create_v1")
public func coreCreate() -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(DesktopContext()).toOpaque()
}

@_cdecl("codeisland_core_copy_v1")
public func coreCopy(_ context: UnsafeMutableRawPointer) -> UnsafeMutableRawPointer {
    Unmanaged.passRetained(Unmanaged<DesktopContext>.fromOpaque(context).takeUnretainedValue().copy()).toOpaque()
}

@_cdecl("codeisland_core_destroy_v1")
public func coreDestroy(_ context: UnsafeMutableRawPointer) {
    Unmanaged<DesktopContext>.fromOpaque(context).release()
}

public typealias WriteBytes = @convention(c) (UnsafeRawPointer?, Int, UnsafeMutableRawPointer?) -> Void

@_cdecl("codeisland_core_call_v1")
public func coreCall(_ context: UnsafeMutableRawPointer?, _ operation: UnsafePointer<CChar>,
                     _ bytes: UnsafeRawPointer, _ count: Int, _ write: WriteBytes,
                     _ destination: UnsafeMutableRawPointer?) -> Int32 {
    let input = Data(bytes: bytes, count: count)
    do {
        let state = context.map { Unmanaged<DesktopContext>.fromOpaque($0).takeUnretainedValue() }
        let result = try dispatch(String(cString: operation), input, state)
        result.withUnsafeBytes { write($0.baseAddress, $0.count, destination) }
        return 0
    } catch {
        let message = Data(String(describing: error).utf8)
        message.withUnsafeBytes { write($0.baseAddress, $0.count, destination) }
        return 1
    }
}

private enum BridgeError: Error { case unknownOperation(String), missingContext, invalidReminder }

private func dispatch(_ op: String, _ data: Data, _ state: DesktopContext?) throws -> Data {
    let decoder = JSONDecoder()
    let encoder = JSONEncoder()
    switch op {
    case "claude.readRecord", "claude.recordID", "codex.storeRows", "codex.rolloutTurn", "codex.header", "codex.origin", "codex.visibleStatus":
        return try discoveryOperation(op, data)
    case "codex.rpc", "codex.questions", "codex.threadStatus", "codex.questionResult":
        return try codexMessage(op, data)
    case let operation where operation.hasPrefix("ssh."):
        guard let state else { throw BridgeError.missingContext }
        return try reconnectOperation(operation, data, policy: &state.reconnect)
    case "codex.enableHooks":
        return try encoder.encode(CodexHooksConfig.enablingHooks(in: decoder.decode(String.self, from: data)))
    case "codex.prefixes", "codex.matches", "codex.scope", "codex.persist":
        return try codexRule(op, data)
    case "hook.seenCursor":
        struct Delivery: Decodable { let digest: String; let now: Int64 }
        guard let state else { throw BridgeError.missingContext }
        let delivery = try decoder.decode(Delivery.self, from: data)
        let date = Date(timeIntervalSince1970: Double(delivery.now) / 1_000_000)
        return try encoder.encode(state.cursor.isRepeat(digest: delivery.digest, now: date))
    case "hook.admit", "hook.prepare": return try prepareHook(op, data)
    case "git.branch":
        let cwd = try decoder.decode(String.self, from: data)
        return try encoder.encode(GitBranchReader.read(cwd: cwd, maxDepth: 64)?.branch)
    case "companion.frames": return try companionFrames(data)
    case let operation where operation.hasPrefix("buddy."): return try buddy(operation, data)
    case "pushEndpoint":
        let endpoint = PushEndpoint.parse(try decoder.decode(String.self, from: data))
        return try encoder.encode(endpoint.map { [$0.url.absoluteString, $0.authorization] })
    case "push.retry", "push.redirect": return try pushTransportPolicy(op, data)
    case "push.admit", "push.slot":
        guard let state else { throw BridgeError.missingContext }
        return try state.pushPolicy.apply(op, data)
    case "push.rateLimits":
        return try encoder.encode(Dictionary(uniqueKeysWithValues: PushChannelKind.allCases.map { kind in
            (kind.rawValue, kind.rateLimits.map { [$0.count, Int($0.seconds)] })
        }))
    case "push.presence":
        struct Presence: Decodable { let away: Bool; let asleep: Bool; let idle: Double; let threshold: Double }
        let args = try decoder.decode(Presence.self, from: data)
        return try encoder.encode(PushPresenceSnapshot(screenLocked: args.away, displaysAsleep: args.asleep,
            idleSeconds: args.idle).isAway(idleThreshold: args.threshold))
    case "push.content": return try pushContent(data)
    case "push.displayWaits": return try displayWaits(data)
    case "renderPush": return try renderPush(data)
    case "acceptPush": return try acceptPush(data)
    case "truncatePushText":
        let args = try decoder.decode(TextLimit.self, from: data)
        return try encoder.encode(PushMessageFormatter.truncated(args.text, limit: args.limit))
    case "sound.quiet":
        struct Clock: Decodable { let minute: Int; let start: Int; let end: Int }
        let clock = try decoder.decode(Clock.self, from: data)
        return try encoder.encode(EventSoundRouting.isInQuietHours(minutesSinceMidnight: clock.minute, start: clock.start, end: clock.end))
    case "sound.failure":
        struct Hit: Decodable { let id: String; let millis: Int64 }
        guard let state else { throw BridgeError.missingContext }
        let hit = try decoder.decode(Hit.self, from: data)
        return try encoder.encode(state.failureSounds.shouldPlay(key: hit.id, now: Date(timeIntervalSince1970: Double(hit.millis) / 1000)))
    case "attention.intents", "attention.decide": return try attentionEvent(data, settle: op == "attention.decide")
    case "attention.settle": return try settleAttention(data)
    case "attention.hold": return try attentionHold(data)
    case "cowork.audit":
        guard let state else { throw BridgeError.missingContext }
        return try coworkBatch(data, state: &state.cowork)
    case "cowork.metadata": return try coworkMetadata(data)
    case "cowork.id": return try encoder.encode(CoworkPaths.isValidSessionId(decoder.decode(String.self, from: data)))
    case "cowork.link": return try encoder.encode(CoworkSessionPolicy.deepLinkURL(sessionId: decoder.decode(String.self, from: data))?.absoluteString)
    case "cowork.trackable":
        let fields = try decoder.decode(CoworkMetadataFields.self, from: data)
        let metadata = CoworkSessionMetadata(sessionId: fields.id, isArchived: fields.archived, sessionType: fields.type)
        return try encoder.encode(CoworkSessionPolicy.isTrackable(metadata))
    case "transcript.kind": return try classifyTranscript(data)
    case "session.checklist":
        guard let state else { throw BridgeError.missingContext }
        return try state.sessions.applyChecklist(data)
    case "session.tier", "session.rank", "session.order", "session.idleGroups":
        return try sessionList(op, data, state?.sessions.cards ?? [:])
    case "session.reduce":
        guard let state else { throw BridgeError.missingContext }
        return try state.sessions.apply(data)
    case "session.remove":
        guard let state else { throw BridgeError.missingContext }
        state.sessions.cards.removeValue(forKey: try decoder.decode(String.self, from: data))
        return Data("null".utf8)
    case "toolStep":
        return try toolStep(data)
    case "cleanPushText":
        let args = try decoder.decode(PushTextArguments.self, from: data)
        return try encoder.encode(PushMessageFormatter.clean(args.text, limit: args.limit, markdown: args.markdown) ?? "")
    case "normalizeEvent":
        return try encoder.encode(EventNormalizer.normalize(decoder.decode(String.self, from: data)))
    case "claudeHostID":
        return try encoder.encode(ClaudeDesktopCodeSession.isValidHostSessionId(decoder.decode(String.self, from: data)))
    case "modelLabel":
        let args = try decoder.decode(ModelArguments.self, from: data)
        return try encoder.encode(ModelLabel.label(model: args.model, effort: args.effort))
    case "scanUsage":
        guard let state else { throw BridgeError.missingContext }
        let args = try decoder.decode(ScanArguments.self, from: data)
        let result = ClaudeUsageScanner.scan(claudeHomes: args.roots, now: args.date, cache: &state.usage)
        encoder.keyEncodingStrategy = .convertToSnakeCase
        return try encoder.encode(state.usage.isEmpty ? nil : UsageReply(result))
    case "fetchQuota":
        return try fetchQuota(in: decoder.decode(String.self, from: data))
    case "parseQuota":
        // Malformed provider responses are a normal client result.
        let snapshot = try? ClaudeQuotaSnapshot.parse(data)
        return try encoder.encode(snapshot.map { $0.limits.map(QuotaRow.init) })
    default:
        guard let state else { throw BridgeError.missingContext }
        if op.hasPrefix("quota.") {
            let time = try decoder.decode(Double.self, from: data)
            let now = Date(timeIntervalSinceReferenceDate: time)
            switch op {
            case "quota.stop": state.quota.recordStop(now: now)
            case "quota.start": state.quota.recordFetchStart(now: now)
            case "quota.success": state.quota.recordSuccess()
            case "quota.unauthorized": state.quota.recordFailure(unauthorized: true)
            case "quota.failure": state.quota.recordFailure(unauthorized: false)
            case "quota.next":
                return try encoder.encode(state.quota.nextFireTime(wantsLive: true, now: now)?.timeIntervalSinceReferenceDate)
            case "quota.expand": return try encoder.encode(state.quota.shouldFetchOnExpand(now: now))
            default: throw BridgeError.unknownOperation(op)
            }
        } else {
            switch op {
            case "reminder.interval":
                state.reminders.setInterval(try decoder.decode(Double?.self, from: data))
            case "reminder.enabled": return try encoder.encode(state.reminders.isEnabled)
            case "reminder.owed": return try encoder.encode(state.reminders.hasOwed)
            case "reminder.next": return try encoder.encode(state.reminders.nextWakeDate().map(microseconds))
            case "reminder.sync":
                let args = try decoder.decode(Waiting.self, from: data)
                if args.origin == .island {
                    state.reminders.sync(kind: args.kind, requests: args.waiting.compactMapValues { $0 }, now: args.date)
                } else {
                    state.reminders.sync(kind: args.kind, origin: args.origin, waiting: Set(args.waiting.keys), now: args.date)
                }
            case "reminder.restart", "reminder.track":
                let args = try decoder.decode(Track.self, from: data)
                if op == "reminder.restart" {
                    state.reminders.restart(kind: args.kind, sessionId: args.id, origin: .displayOnly, now: args.date)
                } else {
                    state.reminders.track(kind: args.kind, sessionId: args.id, now: args.date, turnFailed: args.failed)
                }
            case "reminder.silenceSession":
                state.reminders.silenceAll(sessionId: try decoder.decode(String.self, from: data))
            case "reminder.silenceCompletion":
                state.reminders.silence(.init(.completion, try decoder.decode(String.self, from: data)))
            case "reminder.keepCompletions":
                let keep = try decoder.decode(Set<String>.self, from: data)
                for key in state.reminders.trackedKeys where key.kind == .completion && !keep.contains(key.sessionId) {
                    state.reminders.silence(key)
                }
            case "reminder.current":
                let item = try decoder.decode(ReminderRow.self, from: data)
                return try encoder.encode(state.reminders.isCurrent(item.reminder))
            case "reminder.postpone":
                let args = try decoder.decode(Postpone.self, from: data)
                state.reminders.postpone(args.item.reminder, now: args.date)
            case "reminder.collect":
                let args = try decoder.decode(Collect.self, from: data)
                return try encoder.encode(state.reminders.collectDue(now: args.date, heldBack: args.held).map(ReminderRow.init))
            default: throw BridgeError.unknownOperation(op)
            }
        }
        return Data("null".utf8)
    }
}

struct QuotaRow: Encodable {
    let kind: String
    let percent: Double
    let severity: String
    let resets_at: Int64?
    let scope_label: String?

    init(_ limit: ClaudeQuotaLimit) {
        kind = limit.kind.rawValue
        percent = limit.percent
        severity = limit.severity
        resets_at = limit.resetsAt.map(microseconds)
        scope_label = limit.scopeLabel
    }
}

func microseconds(_ date: Date) -> Int64 {
    Int64((date.timeIntervalSince1970 * 1_000_000).rounded())
}

private protocol Timed { var now: Int64 { get } }
private extension Timed {
    var date: Date { Date(timeIntervalSince1970: Double(now) / 1_000_000) }
}

private struct Waiting: Decodable, Timed {
    let kind: FollowUpReminderKind
    let origin: FollowUpReminder.Origin
    let now: Int64
    let waiting: [String: String?]
}
private struct Track: Decodable, Timed {
    let kind: FollowUpReminderKind
    let id: String
    let now: Int64
    let failed: Bool
}
private struct Collect: Decodable, Timed { let now: Int64; let held: Bool }
private struct Postpone: Decodable, Timed { let now: Int64; let item: ReminderRow }

private struct ReminderRow: Codable {
    struct Key: Codable {
        let kind: FollowUpReminderKind
        let session_id: String
    }
    let key: Key
    let waiting_since: Int64
    let attempt: Int
    let origin: FollowUpReminder.Origin
    let request_id: String?
    let delivery: FollowUpReminder.Delivery
    let turn_failed: Bool

    init(_ item: FollowUpReminder) {
        key = Key(kind: item.kind, session_id: item.sessionId)
        waiting_since = microseconds(item.waitingSince)
        attempt = item.attempt
        origin = item.origin
        request_id = item.requestId
        delivery = item.delivery
        turn_failed = item.turnFailed
    }

    var reminder: FollowUpReminder {
        FollowUpReminder(kind: key.kind, sessionId: key.session_id, attempt: attempt,
                         maxAttempts: key.kind.maxAttempts,
                         waitingSince: Date(timeIntervalSince1970: Double(waiting_since) / 1_000_000),
                         delivery: delivery, origin: origin, requestId: request_id, turnFailed: turn_failed)
    }
}

private struct ModelArguments: Decodable {
    let model: String?
    let effort: String?
}

private struct ScanArguments: Decodable, Timed {
    let roots: [String]
    let now: Int64
}

private struct UsageReply: Encodable {
    let last_5h: ClaudeUsageTotals
    let today: ClaudeUsageTotals
    let hourlyOutputTokens: [Int]

    init(_ scan: ClaudeUsageScanner.Snapshot) {
        last_5h = scan.last5h
        today = scan.today
        hourlyOutputTokens = scan.hourlyOutputTokens
    }
}

private struct CoworkMetadataFields: Decodable {
    let id: String
    let archived: Bool
    let type: String?
}

private struct TextLimit: Decodable { let text: String; let limit: Int }

private struct PushTextArguments: Decodable {
    let text: String
    let limit: Int
    let markdown: Bool
}

func bridgeHook(_ data: Data) throws -> HookEvent? {
    guard let arguments = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
    var payload = arguments["payload"] as? [String: Any] ?? [:]
    // The Windows hook may receive its event and tool name as CLI arguments.
    if let name = arguments["event"] as? String { payload["hook_event_name"] = name }
    if let tool = arguments["tool"] as? String { payload["tool_name"] = tool }
    if let id = arguments["tool_id"] as? String { payload["tool_use_id"] = id }
    if payload["_source"] == nil { payload["_source"] = arguments["source"] }
    if payload["cwd"] == nil, let cwd = arguments["cwd"] as? String { payload["cwd"] = cwd }
    // For a real hook the Swift reducer reads the provider's original fields,
    // including nested replies. Only synthetic desktop events need text hints.
    if arguments["payload"] as? [String: Any] == nil {
        payload["prompt"] = arguments["prompt"] as? String
        payload["last_assistant_message"] = arguments["answer"] as? String
    }
    if let card = arguments["card"] as? String, payload["session_id"] == nil { payload["session_id"] = card }
    return HookEvent(json: payload, cardSessionId: arguments["card"] as? String)
}
