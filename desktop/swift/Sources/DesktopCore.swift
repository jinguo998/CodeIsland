import CodeIslandCore
import Foundation

// The caller owns each context and serializes access to it. Output bytes are
// borrowed only during the callback, so neither runtime frees the other's heap.
private final class DesktopContext {
    var reminders = FollowUpReminderScheduler()
    // Revalidate an asynchronous push against the exact native delivery it received.
    var pendingDeliveries: [FollowUpReminderScheduler.Key: FollowUpReminder] = [:]
    var quota = ClaudeQuotaScheduler()
    var usage = ClaudeUsageScanner.FileCache()
    var sessions = DesktopSessions()
    var cowork = CoworkAuditState()
    var cursor = CursorHookDeliveryFilter()
    var reconnect = RemoteReconnectPolicy()
    var reachabilityUnknown = true
    var pushPolicy = DesktopPushPolicy()
    var failureSounds = DesktopSoundTiming()

    func copy() -> DesktopContext {
        let result = DesktopContext()
        result.reminders = reminders
        result.pendingDeliveries = pendingDeliveries
        result.quota = quota
        result.usage = usage
        result.sessions = sessions
        result.cowork = cowork
        result.cursor = cursor
        result.reconnect = reconnect
        result.reachabilityUnknown = reachabilityUnknown
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
        return try reconnectOperation(operation, data, policy: &state.reconnect, unknown: &state.reachabilityUnknown)
    case "codex.enableHooks":
        return try encoder.encode(CodexHooksConfig.enablingHooks(in: decoder.decode(String.self, from: data)))
    case let operation where operation.hasPrefix("install."): return try installerOperation(operation, data)
    case "codex.prefixes", "codex.matches", "codex.scope", "codex.persist":
        return try codexRule(op, data)
    case "hook.seenCursor":
        struct Delivery: Decodable { let digest: String; let now: Int64 }
        guard let state else { throw BridgeError.missingContext }
        let delivery = try decoder.decode(Delivery.self, from: data)
        let date = Date(timeIntervalSince1970: Double(delivery.now) / 1_000_000)
        return try encoder.encode(state.cursor.isRepeat(digest: delivery.digest, now: date))
    case "tool.fields", "tool.sameInput", "tool.keepsRequest", "tool.hasPassed": return try permissionToolCall(op, data)
    case "hook.admit", "hook.prepare": return try prepareHook(op, data)
    case "lan.digest":
        struct Digest: Decodable { let key: [UInt8]?; let message: [UInt8] }
        let args = try decoder.decode(Digest.self, from: data)
        return try encoder.encode(DesktopLANDigest.bytes(args.message, key: args.key))
    case "desktop.animation":
        return try encoder.encode(NotchAnimationSpeed.clamped((try decoder.decode(Double?.self, from: data)) ?? NotchAnimationSpeed.normal))
    case "desktop.diagnostics":
        return try JSONSerialization.data(withJSONObject: DiagnosticsRedaction.redact(
            try JSONSerialization.jsonObject(with: data)), options: [.withoutEscapingSlashes, .sortedKeys])
    case "desktop.codexLink":
        var session = SessionSnapshot()
        session.source = "codex"
        session.termBundleId = AppState.codexAppBundleId
        session.providerSessionId = try decoder.decode(String.self, from: data)
        return try encoder.encode(AppState.codexDesktopThreadLink(for: session)?.absoluteString)
    case "session.revives":
        guard let event = try bridgeHook(data) else { return try encoder.encode(false) }
        return try encoder.encode(RemovedSessionRevivalEvidence.forHookEvent(event,
            normalizedEventName: EventNormalizer.normalize(event.eventName)) != .passive)
    case "desktop.cardShortcut": return try DesktopCardShortcut.route(data)
    case "tray.mask": return try encoder.encode(DesktopTrayDrawing.mask())
    case "session.project":
        var session = SessionSnapshot()
        session.cwd = try DesktopPath.withForwardSlashes(decoder.decode(String.self, from: data))
        return try encoder.encode(session.displayName)
    case "hook.receive", "hook.fields", "hook.localOrigin", "hook.policy", "hook.remoteFilter",
         "hook.webhook", "hook.answer", "hook.keepsRule", "hook.source":
        return try desktopHookOperation(op, data)
    case "git.branch":
        let cwd = try decoder.decode(String.self, from: data)
        return try encoder.encode(GitBranchReader.desktopBranch(cwd: cwd))
    case "companion.frames": return try companionFrames(data)
    case "companion.state": return try DesktopCompanion.state(data)
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
        return try encoder.encode(DesktopSoundTiming.isInQuietHours(minutesSinceMidnight: clock.minute, start: clock.start, end: clock.end))
    case "sound.failure":
        struct Hit: Decodable { let id: String; let millis: Int64 }
        guard let state else { throw BridgeError.missingContext }
        let hit = try decoder.decode(Hit.self, from: data)
        return try encoder.encode(state.failureSounds.shouldPlay(hit.id, now: Date(timeIntervalSince1970: Double(hit.millis) / 1000)))
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
        return try encoder.encode(state.usage.hasScannedFiles ? UsageReply(result) : nil)
    case "fetchQuota":
        return try fetchQuota(in: decoder.decode(String.self, from: data))
    case "parseQuota":
        // Malformed provider responses are a normal client result.
        let snapshot = try? ClaudeQuotaSnapshot.parse(data)
        let limits = snapshot?.limits.filter { $0.percent.isFinite }
        return try encoder.encode(limits.flatMap { $0.isEmpty ? nil : $0.map(QuotaRow.init) })
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
                if !state.reminders.isEnabled { state.pendingDeliveries.removeAll() }
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
                state.pendingDeliveries = state.pendingDeliveries.filter { key, item in
                    guard state.reminders.origin(of: key) != nil else { return false }
                    if key.kind == args.kind, let request = args.waiting[key.sessionId] ?? nil {
                        return request == item.requestId
                    }
                    return true
                }
            case "reminder.restart", "reminder.track":
                let args = try decoder.decode(Track.self, from: data)
                state.pendingDeliveries.removeValue(forKey: .init(args.kind, args.id))
                if op == "reminder.restart" {
                    state.reminders.restart(kind: args.kind, sessionId: args.id, origin: .displayOnly, now: args.date)
                } else {
                    state.reminders.track(kind: args.kind, sessionId: args.id, now: args.date, turnFailed: args.failed)
                }
            case "reminder.silenceSession":
                let id = try decoder.decode(String.self, from: data)
                state.reminders.silenceAll(sessionId: id)
                state.pendingDeliveries = state.pendingDeliveries.filter { $0.key.sessionId != id }
            case "reminder.silenceCompletion":
                let key = FollowUpReminderScheduler.Key(.completion, try decoder.decode(String.self, from: data))
                state.reminders.silence(key)
                state.pendingDeliveries.removeValue(forKey: key)
            case "reminder.keepCompletions":
                let keep = try decoder.decode(Set<String>.self, from: data)
                for key in state.reminders.trackedKeys where key.kind == .completion && !keep.contains(key.sessionId) {
                    state.reminders.silence(key)
                    state.pendingDeliveries.removeValue(forKey: key)
                }
            case "reminder.current":
                let item = try decoder.decode(ReminderRow.self, from: data)
                return try encoder.encode(state.pendingDeliveries[item.nativeKey].map(ReminderRow.init) == item)
            case "reminder.postpone":
                let args = try decoder.decode(Postpone.self, from: data)
                if let delivered = state.pendingDeliveries[args.item.nativeKey], ReminderRow(delivered) == args.item,
                   delivered.delivery != .deferred {
                    state.reminders.postpone(delivered, now: args.date)
                    state.pendingDeliveries.removeValue(forKey: args.item.nativeKey)
                }
            case "reminder.collect":
                let args = try decoder.decode(Collect.self, from: data)
                let due = state.reminders.collectDue(now: args.date, heldBack: args.held)
                for item in due { state.pendingDeliveries[.init(item.kind, item.sessionId)] = item }
                return try encoder.encode(due.map(ReminderRow.init))
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

private struct ReminderRow: Equatable, Codable {
    struct Key: Equatable, Codable {
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

    var nativeKey: FollowUpReminderScheduler.Key { .init(key.kind, key.session_id) }
}

private struct ModelArguments: Decodable {
    let model: String?
    let effort: String?
}

private struct ScanArguments: Decodable, Timed {
    let roots: [String]
    let now: Int64
}

private struct UsageCounts: Encodable {
    let inputTokens: Int
    let outputTokens: Int
    let cacheCreationTokens: Int
    let cacheReadTokens: Int
    let messageCount: Int

    init(_ totals: ClaudeUsageTotals) {
        inputTokens = totals.inputTokens
        outputTokens = totals.outputTokens
        cacheCreationTokens = totals.cacheCreationTokens
        cacheReadTokens = totals.cacheReadTokens
        messageCount = totals.messageCount
    }
}

private struct UsageReply: Encodable {
    let last_5h: UsageCounts
    let today: UsageCounts
    let hourlyOutputTokens: [Int]

    init(_ scan: ClaudeUsageScanner.Snapshot) {
        last_5h = UsageCounts(scan.last5h)
        today = UsageCounts(scan.today)
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

func bridgeHook(_ data: Data, eventName: String? = nil, forDisplay: Bool = false) throws -> HookEvent? {
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
    if let eventName { payload["hook_event_name"] = eventName }
    guard let event = HookEvent(json: payload) else { return nil }
    guard forDisplay else { return event }
    if var input = event.toolInput {
        for key in ["file_path", "AbsolutePath", "TargetFile", "SearchPath", "path", "filePath"] {
            if let path = input[key] as? String { input[key] = DesktopPath.withForwardSlashes(path) }
        }
        payload["tool_input"] = input
    }
    if let path = payload["transcript_path"] as? String {
        payload["transcript_path"] = DesktopPath.withForwardSlashes(path)
    }
    return HookEvent(json: payload)
}
