import CodeIslandCore
import Foundation

private struct RequestArguments: Decodable {
    let config: PushChannelConfig
    let kind: PushEventKind
    let title: String
    let headline: String
    let body: String
    let now: Double
}

private struct RenderedRequest: Encodable {
    let url: String
    let headers: [String: String]
    let body: String
}

private struct RequestResult: Encodable {
    var request: RenderedRequest?
    var problem: String?
}

func renderPush(_ data: Data) throws -> Data {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    let args = try decoder.decode(RequestArguments.self, from: data)
    let message = PushMessage(kind: args.kind, sessionId: "", title: args.title,
                              headline: args.headline, body: args.config.includeDetails ? args.body : "")
    do {
        let request = try PushRequestBuilder.request(for: message, channel: args.config,
                                                     now: Date(timeIntervalSince1970: args.now))
        return try JSONEncoder().encode(RequestResult(request: .init(url: request.url.absoluteString,
            headers: request.headers, body: String(decoding: request.body, as: UTF8.self))))
    } catch let problem as PushConfigProblem {
        return try JSONEncoder().encode(RequestResult(problem: problem.rawValue))
    }
}

private struct ResponseArguments: Decodable {
    let channel: PushChannelKind
    let status: Int
    let body: String
}

func acceptPush(_ data: Data) throws -> Data {
    let args = try JSONDecoder().decode(ResponseArguments.self, from: data)
    let result = PushResponseInterpreter.interpret(kind: args.channel, statusCode: args.status, body: Data(args.body.utf8))
    return try JSONEncoder().encode(result.ok)
}

func pushTransportPolicy(_ operation: String, _ data: Data) throws -> Data {
    let decoder = JSONDecoder()
    let encoder = JSONEncoder()
    if operation == "push.redirect" {
        struct Redirect: Decodable { let from: URL; let to: URL; let count: Int }
        let args = try decoder.decode(Redirect.self, from: data)
        return try encoder.encode(PushRedirectPolicy.refusal(from: args.from, to: args.to, redirectCount: args.count) == nil)
    }
    struct Failure: Decodable { let kind: PushEventKind; let status: Int?; let after: String?; let body: String }
    let args = try decoder.decode(Failure.self, from: data)
    let response = PushTransportResponse(statusCode: args.status, body: Data(args.body.utf8),
                                        retryAfter: URLSessionPushTransport.desktopRetryAfter(args.after))
    let delay = PushRetryPolicy.retries(args.kind) ? PushRetryPolicy.delay(after: response) : nil
    return try encoder.encode(delay)
}

private struct PushTextRequest: Decodable {
    struct Question: Decodable { let question: String; let header: String?; let options: [String]; let is_secret: Bool }
    struct Request: Decodable { let tool: String?; let title: String?; let body: String?; let options: [String]; let questions: [Question] }
    let kind: String
    let id: String
    let agent: String
    let project: String?
    let language: String
    let limit: Int
    let request: Request?
    let tool: String?
    let text: String?
    let error: String?
    let terminal: String?
    let displayOnly: Bool
    let since: Int64?
    let now: Int64
}

private struct PushText: Encodable {
    let title: String
    let headline: String
    let body: String

    init(_ message: PushMessage) {
        title = message.title
        headline = message.headline
        body = message.body
    }
}

private func desktopPushStrings(_ requested: String) -> PushStrings {
    let tag = requested == "system" ? Locale.preferredLanguages.first ?? "en" : requested
    let traditional = ["zh-Hant", "zh-TW", "zh-HK", "zh-MO"].contains(where: tag.hasPrefix)
    let language = traditional ? "zh-Hant" : String(tag.split(separator: "-").first ?? "en")
    var strings = DesktopPushStrings.strings(L10n(language: language))
    if language == "tr" {
        strings.secretQuestion = "Hassas istem — bilgisayarınızda yanıtlayın."
        strings.answerOnMac = "Lütfen bilgisayarınızdan yanıt verin."
    } else {
        strings.secretQuestion = strings.secretQuestion.replacingOccurrences(of: "Mac", with: "PC")
        strings.answerOnMac = strings.answerOnMac.replacingOccurrences(of: "Mac", with: "PC")
    }
    return strings
}

func pushContent(_ data: Data) throws -> Data {
    let args = try JSONDecoder().decode(PushTextRequest.self, from: data)
    let subject = PushSubject(sessionId: args.id, agent: args.agent, project: PushMessageFormatter.clean(args.project, limit: 80))
    let strings = desktopPushStrings(args.language)
    let request = args.request
    var content: PushContent
    switch args.kind {
    case "approval":
        content = .permission(tool: PushMessageFormatter.clean(request?.tool ?? args.tool, limit: 100), detail: request?.body ?? args.text)
    case "question", "display_only_wait":
        var items = request?.questions.map { PushQuestionItem(question: $0.question, options: $0.options, header: $0.header) } ?? []
        if items.isEmpty, let question = request?.body ?? request?.title ?? args.text {
            items.append(PushQuestionItem(question: question, options: request?.options ?? []))
        }
        content = .question(items: items, isSecret: request?.questions.contains(where: { $0.is_secret }) == true)
    case "error":
        content = .error(type: PushMessageFormatter.clean(args.error, limit: 80), detail: args.text)
    default:
        content = .completion(summary: args.text)
    }
    if args.displayOnly { content = .answerElsewhere(pending: content, app: args.terminal) }
    if let since = args.since {
        content = .reminder(pending: content, waitingSince: Date(timeIntervalSince1970: Double(since) / 1000))
    }
    let now = Date(timeIntervalSince1970: Double(args.now) / 1000)
    func render(_ details: Bool) -> PushText {
        PushText(PushMessageFormatter.render(content, subject: subject, strings: strings, summaryLimit: args.limit,
                                             includeDetails: details, now: now))
    }
    return try JSONEncoder().encode(["full": render(true), "brief": render(false)])
}

func displayWaits(_ data: Data) throws -> Data {
    struct Args: Decodable { let sessions: [String: String]; let queued: Set<String> }
    let args = try JSONDecoder().decode(Args.self, from: data)
    let sessions = args.sessions.mapValues { status in
        var snapshot = SessionSnapshot()
        snapshot.status = nativeStatus(status)
        return snapshot
    }
    return try JSONEncoder().encode(["approval": DisplayOnlyWait.sessionIds(waitingOn: .approval, in: sessions, islandRequestSessionIds: args.queued),
                                     "question": DisplayOnlyWait.sessionIds(waitingOn: .question, in: sessions, islandRequestSessionIds: args.queued)])
}

struct DesktopPushPolicy {
    private var dedup = PushDeduplicator()
    private var global = PushRateLimiter(windows: [PushThrottle.global])
    private var channels: [PushChannelKind: PushRateLimiter] = [:]

    mutating func apply(_ operation: String, _ data: Data) throws -> Data {
        let decoder = JSONDecoder()
        let encoder = JSONEncoder()
        if operation == "push.admit" {
            struct Event: Decodable {
                let kind: PushEventKind; let session: String; let key: String?; let now: Double; let window: Double?
            }
            let e = try decoder.decode(Event.self, from: data)
            let now = Date(timeIntervalSinceReferenceDate: e.now)
            // Compatibility API callers may supply their own single-session cooldown.
            if let window = e.window {
                dedup.window = window
                return try encoder.encode(dedup.admit(kind: e.kind, sessionId: e.session, requestKey: e.key, now: now) == nil)
            }
            dedup.window = PushDeduplicator().window
            let mustDeliver = PushThrottle.mustDeliver(e.kind)
            guard mustDeliver || global.nextSlot(now: now) <= now else { return try encoder.encode(false) }
            guard dedup.admit(kind: e.kind, sessionId: e.session, requestKey: e.key, now: now) == nil else {
                return try encoder.encode(false)
            }
            if !mustDeliver { global.record(now) }
            return try encoder.encode(true)
        }
        struct Slot: Decodable { let channel: String; let mustDeliver: Bool; let now: Double; let waited: Double }
        let slot = try decoder.decode(Slot.self, from: data)
        guard let channel = PushChannelKind(rawValue: slot.channel), !channel.rateLimits.isEmpty else { return Data("0".utf8) }
        let now = Date(timeIntervalSinceReferenceDate: slot.now)
        var limiter = channels[channel] ?? PushRateLimiter(windows: channel.rateLimits)
        let delay = limiter.nextSlot(now: now).timeIntervalSince(now)
        if delay <= 0 {
            limiter.record(now)
            channels[channel] = limiter
            return Data("0".utf8)
        }
        return try encoder.encode(slot.mustDeliver || delay + slot.waited <= PushThrottle.dropGrace ? delay : nil)
    }
}
