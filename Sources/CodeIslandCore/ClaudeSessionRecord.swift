import Foundation

public enum ClaudeSessionRecord {
    /// A record is a few hundred bytes; anything far larger is not one.
    public static let sessionRecordByteLimit = 64 * 1024

    /// The record's bytes, or nil when it can't be read or is too large to be
    /// one. Read only: nothing is opened for writing.
    public static func readSessionRecord(atPath path: String) -> Data? {
        guard let handle = FileHandle(forReadingAtPath: path) else { return nil }
        defer { try? handle.close() }
        guard let data = try? handle.read(upToCount: sessionRecordByteLimit + 1),
              data.count <= sessionRecordByteLimit else { return nil }
        return data
    }

    /// The session id in a record, when it is this process's: the record's
    /// `pid` is `pid` and its `procStart` (UTC, second precision) is the live
    /// process's start, so a record an earlier process with the same pid
    /// left behind is not read as this one's. Anything missing, malformed
    /// or different: nil.
    public static func recordedSessionId(inRecord data: Data, pid: Int64, processStart: Date?) -> String? {
        guard let record = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return recordedSessionId(in: record, pid: pid, processStart: processStart)
    }

    public static func recordedSessionId(in record: [String: Any], pid: Int64, processStart: Date?,
                                         recordedStart: Date? = nil, tolerance: TimeInterval = 1) -> String? {
        guard let processStart,
              (record["pid"] as? NSNumber)?.int64Value == pid,
              let raw = record["procStart"] as? String,
              let recordedStart = recordedStart ?? parseRecordedProcessStart(raw),
              abs(recordedStart.timeIntervalSince(processStart)) <= tolerance,
              let sessionId = record["sessionId"] as? String,
              !sessionId.isEmpty, !sessionId.contains("/") else { return nil }
        return sessionId
    }

    /// `procStart` as Claude Code writes it: "Fri Oct 23 00:59:52 2026", UTC
    /// (a day below 10 may be space-padded).
    public static func parseRecordedProcessStart(_ raw: String) -> Date? {
        let collapsed = raw.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "UTC")
        formatter.dateFormat = "EEE MMM d HH:mm:ss yyyy"
        return formatter.date(from: collapsed)
    }
}
