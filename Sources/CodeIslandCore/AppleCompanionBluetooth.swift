import Foundation

public enum AppleCompanionBluetooth {
    private static let maxChunkPayloadBytes = 120
    private static let headerBytes = 15

    public static func frames(for payload: AppleCompanionStatePayload,
                              maxNotificationBytes: Int? = nil) throws -> [Data] {
        var bodySize = maxChunkPayloadBytes
        if let maximum = maxNotificationBytes {
            guard maximum > headerBytes else { return [] }
            bodySize = min(bodySize, maximum - headerBytes)
        }
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        var summary = AppleCompanionBluetoothSummary(payload: payload)
        while true {
            let data = try encoder.encode(summary)
            if maxNotificationBytes == nil || data.count <= 64 * bodySize {
                return makeChunks(sequence: payload.sequence, data: data, chunkSize: bodySize)
            }
            guard summary.removeOptionalPreview() else { return [] }
        }
    }

    private static func makeChunks(sequence: UInt64, data: Data, chunkSize: Int) -> [Data] {
        let total = max(1, Int(ceil(Double(data.count) / Double(chunkSize))))

        guard total <= Int(UInt16.max) else { return [] }
        return (0..<total).map { index in
            let start = index * chunkSize
            let end = min(start + chunkSize, data.count)
            let body = data.subdata(in: start..<end)

            var chunk = Data()
            chunk.append(0x43)
            chunk.append(0x49)
            chunk.append(0x01)
            chunk.appendUInt64(sequence)
            chunk.appendUInt16(UInt16(index))
            chunk.appendUInt16(UInt16(total))
            chunk.append(body)
            return chunk
        }
    }
}

private struct AppleCompanionBluetoothSummary: Codable {
    struct SessionSummary: Codable {
        let sessionId: String?
        let source: String
        let status: String
        let toolName: String?
        let workspaceName: String?
        let message: String?
        let updatedAt: Date
    }

    let version: Int
    let sequence: UInt64
    var sessionId: String?
    let source: String
    let status: String
    var toolName: String?
    var workspaceName: String?
    var message: String?
    let pendingAction: String?
    var questionHeader: String?
    var questionText: String?
    var sessions: [SessionSummary]
    let updatedAt: Date

    init(payload: AppleCompanionStatePayload) {
        version = 1
        sequence = payload.sequence
        sessionId = payload.sessionId.map { Self.truncate($0, limit: 96) }
        source = payload.source
        status = payload.status.rawValue
        toolName = payload.toolName.map { Self.truncate($0, limit: 64) }
        workspaceName = payload.workspaceName.map { Self.truncate($0, limit: 64) }
        message = payload.messages.last.map { Self.truncate($0.text, limit: 220) }
        pendingAction = payload.pendingAction?.rawValue
        questionHeader = payload.question?.header.map { Self.truncate($0, limit: 40) }
        questionText = payload.question.map { Self.truncate($0.question, limit: 180) }
        sessions = payload.sessions.prefix(5).map {
            SessionSummary(
                sessionId: $0.sessionId.map { Self.truncate($0, limit: 96) },
                source: $0.source,
                status: $0.status.rawValue,
                toolName: $0.toolName.map { Self.truncate($0, limit: 48) },
                workspaceName: $0.workspaceName.map { Self.truncate($0, limit: 48) },
                message: $0.message.map { Self.truncate($0, limit: 120) },
                updatedAt: $0.updatedAt
            )
        }
        updatedAt = payload.updatedAt
    }

    /// Keep the current status on centrals that negotiate a small ATT packet.
    mutating func removeOptionalPreview() -> Bool {
        if sessions.popLast() != nil { return true }
        for field in [\Self.message, \.questionHeader, \.questionText, \.workspaceName, \.toolName, \.sessionId]
            where self[keyPath: field] != nil {
            self[keyPath: field] = nil
            return true
        }
        return false
    }

    private static func truncate(_ text: String, limit: Int) -> String {
        guard text.count > limit else { return text }
        return String(text.prefix(max(0, limit - 1))) + "…"
    }
}

private extension Data {
    mutating func appendUInt16(_ value: UInt16) {
        var bigEndian = value.bigEndian
        Swift.withUnsafeBytes(of: &bigEndian) { append(contentsOf: $0) }
    }

    mutating func appendUInt64(_ value: UInt64) {
        var bigEndian = value.bigEndian
        Swift.withUnsafeBytes(of: &bigEndian) { append(contentsOf: $0) }
    }
}
