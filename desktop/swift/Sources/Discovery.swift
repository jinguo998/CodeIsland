import CodeIslandCore
import Foundation

func discoveryOperation(_ operation: String, _ data: Data) throws -> Data {
    let decoder = JSONDecoder()
    let encoder = JSONEncoder()
    switch operation {
    case "claude.readRecord":
        let path = try decoder.decode(String.self, from: data)
        guard let record = ClaudeSessionRecord.readSessionRecord(atPath: path),
              (try? JSONSerialization.jsonObject(with: record)) is [String: Any] else { return Data("null".utf8) }
        return record
    case "claude.recordID":
        guard let args = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let record = args["record"] as? [String: Any], let pid = args["pid"] as? Int64,
              let started = args["started"] as? Double, let exact = args["exact"] as? Bool else {
            return Data("null".utf8)
        }
        let platformTime = (args["platformTime"] as? Double).map { Date(timeIntervalSince1970: $0 / 1000) }
        let verified = ClaudeSessionRecord.recordedSessionId(in: record, pid: pid,
            processStart: Date(timeIntervalSince1970: started / 1000), recordedStart: platformTime,
            tolerance: exact ? 0 : 1)
        return try encoder.encode(verified)
    case "codex.storeRows":
        struct Store: Decodable { let path: String; let now: Double }
        let args = try decoder.decode(Store.self, from: data)
        return try encoder.encode(CodexStateStore.recentRows(at: args.path, now: Date(timeIntervalSince1970: args.now)))
    case "codex.rolloutTurn":
        let path = try decoder.decode(String.self, from: data)
        let turn = CodexRollout.latestTurnStatus(path: path, requireFinalNewline: true)
        return try encoder.encode(turn.map { $0 == .processing ? "running" : "settled" })
    case "codex.header":
        struct Header: Encodable { let id: String?; let origin: String; let parent: String?; let title: String? }
        let payload = try JSONSerialization.jsonObject(with: data, options: .fragmentsAllowed) as? [String: Any] ?? [:]
        let metadata = CodexRollout.subagent(in: payload).metadata
        let source = payload["source"]
        let origin = CodexRollout.origin(originator: payload["originator"] as? String, source: source as? String,
                                         isSubagentSource: source is [String: Any])
        let title = metadata.flatMap { $0.agentNickname ?? (payload["agent_nickname"] as? String) ?? $0.agentType }
        return try encoder.encode(Header(id: (payload["id"] ?? payload["session_id"]) as? String,
                                         origin: origin.rawValue, parent: metadata?.parentThreadId, title: title))
    case "codex.origin":
        struct Source: Decodable { let originator: String?; let source: String?; let subagent: Bool }
        let args = try decoder.decode(Source.self, from: data)
        return try encoder.encode(CodexRollout.origin(originator: args.originator, source: args.source,
                                                      isSubagentSource: args.subagent).rawValue)
    default:
        struct Times: Decodable { let now: Double; let modified: Double; let running: Bool }
        let args = try decoder.decode(Times.self, from: data)
        let status = CodexStateStore.visibleStatus(modifiedAt: Date(timeIntervalSince1970: args.modified),
            now: Date(timeIntervalSince1970: args.now), turnStatus: args.running ? .processing : .idle)
        return try encoder.encode(status.map { $0 == .processing ? "running" : "idle" })
    }
}
