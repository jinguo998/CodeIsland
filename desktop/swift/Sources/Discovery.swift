import CodeIslandCore
import Foundation

func discoveryOperation(_ operation: String, _ data: Data) throws -> Data {
    let decoder = JSONDecoder()
    let encoder = JSONEncoder()
    switch operation {
    case "claude.readRecord":
        let path = try decoder.decode(String.self, from: data)
        guard let record = ClaudeDiscovery.readSessionRecord(atPath: path),
              (try? JSONSerialization.jsonObject(with: record)) is [String: Any] else { return Data("null".utf8) }
        return record
    case "claude.recordID":
        guard let args = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              var record = args["record"] as? [String: Any], let value = args["pid"] as? Int64,
              let pid = Int32(exactly: value), record["pid"] as? Int64 == value,
              let started = args["started"] as? Double, let exact = args["exact"] as? Bool else {
            return Data("null".utf8)
        }
        var processStart = Date(timeIntervalSince1970: started / 1000)
        if let recorded = args["platformTime"] as? Double {
            guard abs(recorded - started) <= (exact ? 0 : 1000) else { return Data("null".utf8) }
            // Win32 FILETIME and WSL ISO timestamps have finer precision than ps(1).
            // Validate that precision before projecting the OS clock into the original record.
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.timeZone = TimeZone(secondsFromGMT: 0)
            formatter.dateFormat = "EEE MMM d HH:mm:ss yyyy"
            processStart = Date(timeIntervalSince1970: floor(recorded / 1000))
            record["procStart"] = formatter.string(from: processStart)
        } else if exact { return Data("null".utf8) }
        let verified = ClaudeDiscovery.recordedSessionId(inRecord: try JSONSerialization.data(withJSONObject: record),
                                                         pid: pid, processStart: processStart)
        return try encoder.encode(verified)
    case "codex.storeRows":
        struct Store: Decodable { let path: String; let now: Double }
        let args = try decoder.decode(Store.self, from: data)
        return try encoder.encode(CodexStateStore.recentRows(at: args.path, now: Date(timeIntervalSince1970: args.now)))
    case "codex.rolloutTurn":
        let path = try decoder.decode(String.self, from: data)
        let turn = CodexDiscovery.latestCodexTurnStatus(path: path)
        return try encoder.encode(turn.map { $0 == .processing ? "running" : "settled" })
    case "codex.header":
        struct Header: Encodable { let id: String?; let origin: String; let parent: String?; let title: String? }
        let payload = try JSONSerialization.jsonObject(with: data, options: .fragmentsAllowed) as? [String: Any] ?? [:]
        return try withHeader(payload) {
            let metadata = CodexDiscovery.inspectCodexSubagentMetadata(inTranscriptPath: "").metadata
            let title = metadata.flatMap { $0.agentNickname ?? (payload["agent_nickname"] as? String) ?? $0.agentType }
            return try encoder.encode(Header(id: (payload["id"] ?? payload["session_id"]) as? String,
                origin: String(describing: CodexDiscovery.codexTranscriptOrigin(path: "")),
                parent: metadata?.parentThreadId, title: title))
        }
    case "codex.origin":
        struct Source: Decodable { let originator: String?; let source: String?; let subagent: Bool }
        let args = try decoder.decode(Source.self, from: data)
        var payload = [String: Any]()
        payload["originator"] = args.originator
        if args.subagent { payload["source"] = [String: Any]() } else { payload["source"] = args.source }
        return try withHeader(payload) { try encoder.encode(String(describing: CodexDiscovery.codexTranscriptOrigin(path: ""))) }
    default:
        struct Times: Decodable { let now: Double; let modified: Double; let running: Bool }
        let args = try decoder.decode(Times.self, from: data)
        let status = CodexStateStore.visibleStatus(modifiedAt: Date(timeIntervalSince1970: args.modified),
            now: Date(timeIntervalSince1970: args.now), turnStatus: args.running ? .processing : .idle)
        return try encoder.encode(status.map { $0 == .processing ? "running" : "idle" })
    }
}

// Both the SQL rows and WSL transports can already hold the first rollout record.
private func withHeader<T>(_ input: [String: Any], _ read: () throws -> T) throws -> T {
    var payload = input
    if (payload["source"] as? String)?.lowercased() == "app_server" { payload["source"] = "appServer" }
    let bytes = try JSONSerialization.data(withJSONObject: ["type": "session_meta", "payload": payload])
    return try SessionDiscovery.$suppliedHeader.withValue(String(decoding: bytes, as: UTF8.self), operation: read)
}

// Expose only newline-committed records to the original backwards reader.
// The provider may still be writing its last row through a Windows or WSL mount.
struct CompleteRolloutHandle {
    private let file: Foundation.FileHandle
    init?(forReadingAtPath path: String) {
        guard let handle = Foundation.FileHandle(forReadingAtPath: path) else { return nil }
        file = handle
    }
    func seekToEndOfFile() -> UInt64 {
        let end = file.seekToEndOfFile()
        let lower = end - min(end, 64 * 1024 * 1024)
        var cursor = end
        while cursor > lower {
            let count = min(cursor - lower, 65536)
            cursor -= count
            file.seek(toFileOffset: cursor)
            let chunk = file.readData(ofLength: Int(count))
            if let newline = chunk.lastIndex(of: 0x0a) { return cursor + UInt64(newline + 1) }
        }
        return 0
    }
    func seek(toFileOffset position: UInt64) { file.seek(toFileOffset: position) }
    func readData(ofLength count: Int) -> Data { file.readData(ofLength: count) }
    func closeFile() { file.closeFile() }
}
