import Foundation

/// A parsed JSON-RPC envelope received from the Codex app-server.
///
/// The Codex transport is newline-delimited JSON; the fields below match
/// JSON-RPC 2.0 with a tolerant reading of responses (the server omits the
/// `jsonrpc` marker on successful replies, which is fine for clients).
public struct CodexJSONRPCMessage: Equatable {
    public enum Kind: Equatable {
        case request(method: String, id: CodexRequestID)
        case notification(method: String)
        case response(id: CodexRequestID)
        case error(id: CodexRequestID?, code: Int, message: String)
    }

    public let raw: [String: AnyCodableLike]
    public let kind: Kind

    public init(raw: [String: AnyCodableLike], kind: Kind) {
        self.raw = raw
        self.kind = kind
    }
}

/// Matches JSON-RPC's union type for request/response ids: int or string.
public enum CodexRequestID: Hashable {
    case int(Int64)
    case string(String)

    public static func decode(_ value: Any?) -> CodexRequestID? {
        if let n = value as? Int { return .int(Int64(n)) }
        if let n = value as? Int64 { return .int(n) }
        if let n = value as? NSNumber { return .int(n.int64Value) }
        if let s = value as? String { return .string(s) }
        return nil
    }

    public var jsonValue: Any {
        switch self {
        case .int(let n): return NSNumber(value: n)
        case .string(let s): return s
        }
    }
}

/// Lightweight, untyped wrapper around `Any` so message records round-trip
/// through `Equatable` without forcing the full schema to be modeled here.
public enum AnyCodableLike: Equatable {
    case null
    case bool(Bool)
    case int(Int64)
    case double(Double)
    case string(String)
    case array([AnyCodableLike])
    case object([String: AnyCodableLike])

    public static func from(_ value: Any?) -> AnyCodableLike {
        switch value {
        case nil: return .null
        case let v as NSNull: _ = v; return .null
        case let v as Bool: return .bool(v)
        case let v as Int: return .int(Int64(v))
        case let v as Int64: return .int(v)
        case let v as NSNumber:
            let encoding = String(cString: v.objCType)
            if encoding == "f" || encoding == "d" { return .double(v.doubleValue) }
            return .int(v.int64Value)
        case let v as Double: return .double(v)
        case let v as String: return .string(v)
        case let v as [Any]: return .array(v.map(AnyCodableLike.from))
        case let v as [String: Any]:
            var out: [String: AnyCodableLike] = [:]
            for (k, val) in v { out[k] = AnyCodableLike.from(val) }
            return .object(out)
        default: return .null
        }
    }

    public var asObject: [String: AnyCodableLike]? {
        if case .object(let o) = self { return o }
        return nil
    }
    public var asString: String? {
        if case .string(let s) = self { return s }
        return nil
    }
    public var asBool: Bool? {
        if case .bool(let b) = self { return b }
        return nil
    }
}

public struct AskUserQuestionItem {
    public let payload: QuestionPayload
    public let answerKey: String
    public let multiSelect: Bool

    public init(payload: QuestionPayload, answerKey: String, multiSelect: Bool) {
        self.payload = payload
        self.answerKey = answerKey
        self.multiSelect = multiSelect
    }
}

public enum CodexAppServerProtocol {
    /// Consume as many complete newline-delimited JSON messages as possible
    /// from the buffer. Trailing partial bytes remain in the buffer untouched.
    public static func drainMessages(buffer: inout Data) -> [CodexJSONRPCMessage] {
        var results: [CodexJSONRPCMessage] = []
        let newline: UInt8 = 0x0A
        var searchStart = buffer.startIndex

        while searchStart < buffer.endIndex {
            guard let newlineIndex = buffer[searchStart..<buffer.endIndex].firstIndex(of: newline) else {
                break
            }
            let lineBytes = buffer[searchStart..<newlineIndex]
            if !lineBytes.isEmpty, let parsed = parseMessage(Data(lineBytes)) {
                results.append(parsed)
            }
            searchStart = buffer.index(after: newlineIndex)
        }

        if searchStart == buffer.startIndex { return results }
        if searchStart >= buffer.endIndex {
            buffer.removeAll(keepingCapacity: true)
        } else {
            buffer = Data(buffer[searchStart..<buffer.endIndex])
        }
        return results
    }

    /// Turn a single JSON object string into a `CodexJSONRPCMessage`, or return
    /// nil when the payload is not a recognizable envelope.
    public static func parseMessage(_ data: Data) -> CodexJSONRPCMessage? {
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        return parseMessage(obj)
    }

    public static func parseMessage(_ obj: [String: Any]) -> CodexJSONRPCMessage? {
        let raw = AnyCodableLike.from(obj).asObject ?? [:]

        if let errorPayload = obj["error"] as? [String: Any] {
            let code = (errorPayload["code"] as? Int)
                ?? (errorPayload["code"] as? NSNumber).map(\.intValue)
                ?? 0
            let message = (errorPayload["message"] as? String) ?? ""
            return CodexJSONRPCMessage(
                raw: raw,
                kind: .error(id: CodexRequestID.decode(obj["id"]), code: code, message: message)
            )
        }

        if let method = obj["method"] as? String {
            if let id = CodexRequestID.decode(obj["id"]) {
                return CodexJSONRPCMessage(raw: raw, kind: .request(method: method, id: id))
            } else {
                return CodexJSONRPCMessage(raw: raw, kind: .notification(method: method))
            }
        }

        if let id = CodexRequestID.decode(obj["id"]) {
            return CodexJSONRPCMessage(raw: raw, kind: .response(id: id))
        }
        return nil
    }

    public static func questions(in params: [String: AnyCodableLike]) -> [AskUserQuestionItem] {
        guard case .array(let rawQuestions)? = params["questions"], !rawQuestions.isEmpty else { return [] }

        var askItems: [AskUserQuestionItem] = []
        var usedAnswerKeys = Set<String>()
        for entry in rawQuestions {
            guard let q = entry.asObject else { continue }
            let questionText = q["question"]?.asString ?? "Question"
            let header = q["header"]?.asString
            let isSecret = q["isSecret"]?.asBool ?? false

            var optionLabels: [String] = []
            var optionDescs: [String] = []
            if case .array(let opts)? = q["options"] {
                for opt in opts {
                    guard let o = opt.asObject else { continue }
                    optionLabels.append(o["label"]?.asString ?? "")
                    optionDescs.append(o["description"]?.asString ?? "")
                }
            }

            // answerKey = question.id (the wire key we must echo in the reply).
            // Fall back to a synthetic key and de-duplicate to keep the wizard's
            // position->answerKey mapping unambiguous.
            var answerKey = q["id"]?.asString ?? questionText
            if answerKey.isEmpty { answerKey = questionText }
            if usedAnswerKeys.contains(answerKey) {
                var suffix = 2
                while usedAnswerKeys.contains("\(answerKey)_\(suffix)") { suffix += 1 }
                answerKey = "\(answerKey)_\(suffix)"
            }
            usedAnswerKeys.insert(answerKey)

            let payload = QuestionPayload(
                question: questionText,
                options: optionLabels.isEmpty ? nil : optionLabels,
                descriptions: optionDescs.contains(where: { !$0.isEmpty }) ? optionDescs : nil,
                header: header,
                isSecret: isSecret
            )
            askItems.append(AskUserQuestionItem(payload: payload, answerKey: answerKey, multiSelect: false))
        }
        return askItems
    }

    public static func questionResult(answersByKey: [String: [String]]?) -> [String: Any] {
        var answers: [String: Any] = [:]
        if let answersByKey {
            for (key, values) in answersByKey {
                answers[key] = ["answers": values]
            }
        }
        return ["answers": answers]
    }

    @discardableResult
    public static func applyThreadStatus(_ snapshot: inout SessionSnapshot, status: [String: AnyCodableLike]?) -> Bool {
        guard let typeLabel = status?["type"]?.asString else { return false }
        switch typeLabel {
        case "active":
            let flags: [AnyCodableLike]
            if case .array(let a) = status?["activeFlags"] ?? .null { flags = a } else { flags = [] }
            let flagStrings = flags.compactMap { $0.asString }
            if flagStrings.contains("waitingOnApproval") {
                snapshot.status = .waitingApproval
            } else if flagStrings.contains("waitingOnUserInput") {
                snapshot.status = .waitingQuestion
            } else {
                snapshot.status = .running
                snapshot.currentTool = nil
                snapshot.toolDescription = nil
            }
        case "idle":
            snapshot.status = .idle
            snapshot.currentTool = nil
            snapshot.toolDescription = nil
        case "systemError", "notLoaded":
            snapshot.status = .idle
        default:
            return false
        }
        return true
    }
}
