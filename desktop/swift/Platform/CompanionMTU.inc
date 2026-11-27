    static func frames(for payload: AppleCompanionStatePayload, maxNotificationBytes: Int) throws -> [Data] {
        let header = 15
        guard maxNotificationBytes > header else { return [] }
        let size = min(Limits.maximum, maxNotificationBytes - header)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        var bytes = try encoder.encode(AppleCompanionBluetoothSummary(payload: payload))
        if bytes.count > 64 * size {
            guard var object = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else { return [] }
            var sessions = object["sessions"] as? [[String: Any]] ?? []
            let optional = ["message", "questionHeader", "questionText", "workspaceName", "toolName", "sessionId"]
            while bytes.count > 64 * size {
                if sessions.popLast() != nil { object["sessions"] = sessions }
                else if let key = optional.first(where: { object[$0] != nil }) { object.removeValue(forKey: key) }
                else { return [] }
                bytes = try JSONSerialization.data(withJSONObject: object)
            }
        }
        return $maxChunkPayloadBytes.withValue(size) { makeChunks(sequence: payload.sequence, data: bytes) }
    }
