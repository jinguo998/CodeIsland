import CodeIslandCore
import Foundation

func companionFrames(_ data: Data) throws -> Data {
    struct Arguments: Decodable { let payload: AppleCompanionStatePayload; let maximum: Int }
    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    let args = try decoder.decode(Arguments.self, from: data)
    let frames = try DesktopCompanionFrames.frames(for: args.payload, maxNotificationBytes: args.maximum)
    return try JSONEncoder().encode(frames.map { [UInt8]($0) })
}

private enum BuddyError: Error { case invalidFrame }

func buddy(_ operation: String, _ data: Data) throws -> Data {
    let decoder = JSONDecoder()
    let encoder = JSONEncoder()
    let frame: Data
    switch operation {
    case "buddy.source":
        return try encoder.encode(MascotID(sourceName: decoder.decode(String.self, from: data))?.rawValue)
    case "buddy.uplink":
        struct Event: Encodable { let kind: String; let value: UInt8 }
        let result = BuddyUplinkEvent(payload: Data(try decoder.decode([UInt8].self, from: data))).map { event in
            switch event {
            case .focus(let id): return Event(kind: "focus", value: id.rawValue)
            case .command(let command): return Event(kind: "command", value: command.rawValue)
            case .pairResponse(let response): return Event(kind: "pair", value: response.rawValue)
            }
        }
        return try encoder.encode(result)
    case "buddy.agent":
        struct Agent: Decodable { let mascot: UInt8; let status: UInt8; let tool: String? }
        let args = try decoder.decode(Agent.self, from: data)
        guard let id = MascotID(rawValue: args.mascot), let status = MascotStatusCode(rawValue: args.status)
        else { throw BuddyError.invalidFrame }
        frame = MascotFramePayload(mascot: id, status: status, toolName: args.tool).encode()
    case "buddy.workspace":
        frame = try BuddyWorkspacePayload(workspaceName: decoder.decode(String?.self, from: data)).encode()
    case "buddy.preview":
        struct Preview: Decodable { let index: UInt8; let total: UInt8; let user: Bool; let text: String? }
        let args = try decoder.decode(Preview.self, from: data)
        frame = BuddyMessagePreviewPayload(index: args.index, total: args.total, isUser: args.user, text: args.text).encode()
    case "buddy.brightness":
        frame = try BuddyBrightnessPayload(percent: decoder.decode(UInt8.self, from: data)).encode()
    case "buddy.orientation":
        let value = try decoder.decode(UInt8.self, from: data)
        frame = BuddyScreenOrientationPayload(orientation: .init(wireValue: value)).encode()
    case "buddy.model":
        frame = try BuddyModelPayload(modelName: decoder.decode(String?.self, from: data)).encode()
    case "buddy.stats":
        struct Stats: Decodable { let active: Int; let total: Int; let tools: Int; let minutes: Int }
        let args = try decoder.decode(Stats.self, from: data)
        frame = BuddyStatsPayload(activeSessionCount: args.active, totalSessionCount: args.total,
                                  toolCallCount: args.tools, sessionDurationMinutes: args.minutes).encode()
    case "buddy.subagent":
        frame = try BuddySubagentPayload(count: decoder.decode(Int.self, from: data)).encode()
    case "buddy.event":
        frame = try BuddyEventPayload(eventId: decoder.decode(UInt8.self, from: data)).encode()
    case "buddy.time":
        frame = try BuddyTimeHintPayload(hour: decoder.decode(Int.self, from: data)).encode()
    case "buddy.tool":
        struct Tool: Decodable { let index: Int; let success: Bool; let name: String }
        let args = try decoder.decode(Tool.self, from: data)
        frame = BuddyToolHistoryPayload(index: args.index, success: args.success, toolName: args.name).encode()
    case "buddy.clear": frame = BuddyToolHistoryClearPayload().encode()
    case "buddy.pair", "buddy.unpair":
        let host = Data(try decoder.decode([UInt8].self, from: data))
        frame = operation == "buddy.pair" ? BuddyPairRequestPayload(hostId: host).encode() : BuddyUnpairPayload(hostId: host).encode()
    default: throw BuddyError.invalidFrame
    }
    return try encoder.encode([UInt8](frame))
}
