import CodeIslandCore
import Foundation

// The desktop store also persists Windows process/window identities. Pass the
// previous tool fields to the Swift reducer so restores and live events use the
// same state as the card; there is no second in-memory session map.
private struct ToolFields: Codable {
    let tool: String?
    let detail: String?
    let id: String?
    let executing: Bool
}

private struct Arguments: Decodable {
    let previous: ToolFields
    let step: String
}

func toolStep(_ input: Data) throws -> Data {
    let args = try JSONDecoder().decode(Arguments.self, from: input)
    guard let event = try bridgeHook(input) else { return try JSONEncoder().encode(args.previous) }
    var session = SessionSnapshot()
    session.status = args.previous.executing ? .running : .processing
    session.currentTool = args.previous.tool
    session.currentToolUseId = args.previous.id
    session.toolDescription = args.previous.detail
    // This projection contains only the visible tool. The desktop event log
    // stores its own OS timestamp and event identity.
    session.applyToolStep(event, eventName: args.step, maxHistory: 0)
    return try JSONEncoder().encode(ToolFields(tool: session.currentTool, detail: session.toolDescription,
                                               id: session.currentToolUseId, executing: session.status == .running))
}
