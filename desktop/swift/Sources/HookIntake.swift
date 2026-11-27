import CodeIslandCore
import Foundation

func prepareHook(_ operation: String, _ data: Data) throws -> Data {
    struct Arguments: Decodable {
        let source: String
        let event: String
        let raw: String?
        let cwd: String?
        let parent: Int32?
    }
    struct Admission: Encodable { let admitted: Bool; let immediate: String? }
    let args = try JSONDecoder().decode(Arguments.self, from: data)
    // This entry point is invoked by the local hook process. A remote hook
    // sends the facts from its own process over its transport.
    let environment = ProcessInfo.processInfo.environment.filter { HookIntake.environmentKeys.contains($0.key) }
    if operation == "hook.admit" {
        return try JSONEncoder().encode(Admission(
            admitted: HookIntake.admits(sourceTag: args.source, environment: environment),
            immediate: HookIntake.immediateStdout(sourceTag: args.source, eventName: args.event)))
    }
    let raw = Data((args.raw ?? "").utf8)
    let empty = raw.allSatisfy { [9, 10, 13, 32].contains($0) }
    guard var input = empty ? [:] : HookIntake.stdinObject(raw, sourceTag: args.source) else {
        return Data("null".utf8)
    }
    // Some desktop plugins use a numeric session identifier.
    if let number = input["session_id"] as? NSNumber { input["session_id"] = number.stringValue }
    let cursor = CursorHookPayload.isCursorHook(input)
    let tag = args.source == "claude" && cursor ? nil : args.source
    let eventTag = (input["event_name"] as? String) ?? (input["type"] as? String) ?? args.event
    let parent = (input["_ppid"] as? NSNumber)?.int32Value ?? args.parent
    let facts = HookIntake.Facts(sourceTag: tag, eventTag: eventTag, defaultSource: args.source,
        environment: environment, parentPID: parent, workingDirectory: { args.cwd },
        homeDirectory: { FileManager.default.homeDirectoryForCurrentUser.path },
        stdinDigest: { CursorHookPayload.deliveryDigest(of: raw) },
        fileExists: { FileManager.default.fileExists(atPath: $0) })
    guard var intake = HookIntake(stdin: input, facts: facts) else { return Data("null".utf8) }
    intake.addTerminalContext()
    var result: [String: Any] = ["payload": intake.payload, "source": intake.source ?? args.source,
        "fallback": intake.sessionIdIsFallback, "waits": intake.waitsForAnswer, "cursor": cursor,
        "delay": intake.deliveryDelay]
    result["immediate"] = intake.immediateStdout
    return try JSONSerialization.data(withJSONObject: result)
}
