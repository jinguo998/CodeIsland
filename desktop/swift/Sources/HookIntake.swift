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
    if cursor, !intake.waitsForAnswer, let event = HookEvent(json: intake.payload),
       HookServer.routeKind(for: event) != .event { return Data("null".utf8) }
    var result: [String: Any] = ["payload": intake.payload, "source": intake.source ?? args.source,
        "fallback": intake.sessionIdIsFallback, "waits": intake.waitsForAnswer, "cursor": cursor,
        "delay": intake.deliveryDelay]
    result["immediate"] = intake.immediateStdout
    return try JSONSerialization.data(withJSONObject: result)
}

func permissionToolCall(_ operation: String, _ data: Data) throws -> Data {
    let args = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    let encoder = JSONEncoder()
    func input(_ value: Any?) -> Any? { value is NSNull ? nil : value }
    switch operation {
    case "tool.fields":
        var row = args
        // Desktop CLI adapters can supply tool facts without a wire hook name.
        row["hook_event_name"] = "PreToolUse"
        let event = HookEvent(json: row)
        var result: [String: Any] = [:]
        result["id"] = event?.toolUseId
        result["input"] = event?.toolInput ?? input(row["tool_input"] ?? row["toolInput"])
        return try JSONSerialization.data(withJSONObject: result)
    case "tool.sameInput":
        return try encoder.encode(try sameWireInput(input(args["first"]), input(args["second"])))
    case "tool.keepsRequest":
        let name = EventNormalizer.normalize(args["event"] as? String ?? "")
        guard let step = HookEvent(json: ["hook_event_name": name, "_source": args["source"] ?? NSNull()]),
              let request = HookEvent(json: ["hook_event_name": "PermissionRequest", "_source": args["requestSource"] ?? NSNull()])
        else { return try encoder.encode(false) }
        return try encoder.encode(DesktopPermissionCalls().shouldKeepQueuedPermissionForCompletedEvent(
            step, queued: PermissionRequest(event: request), normalizedEventName: name))
    default:
        func call(_ key: String) -> HookEvent? {
            let call = args[key] as? [String: Any] ?? [:]
            return HookEvent(json: ["hook_event_name": "PreToolUse", "_source": "claude",
                "agent_id": call["agent"] ?? NSNull(), "tool_name": call["tool"] ?? NSNull(),
                "tool_input": call["input"] ?? NSNull()])
        }
        guard let step = call("step"), let request = call("request") else { return try encoder.encode(false) }
        let original = input(args["original"])
        let matches = DesktopPermissionCalls.step(step, calledWith: original as? [String: Any], isPastRequest: request)
        let supplied = [input((args["step"] as? [String: Any])?["input"]), original].compactMap { $0 }
        if matches, let asked = input((args["request"] as? [String: Any])?["input"]), !supplied.isEmpty {
            return try encoder.encode(try supplied.contains { try sameWireInput($0, asked) })
        }
        return try encoder.encode(matches)
    }
}

// Preserve JSON boolean and integer identity across the Rust/Foundation boundary.
private func sameWireInput(_ left: Any?, _ right: Any?) throws -> Bool {
    let options: JSONSerialization.WritingOptions = [.sortedKeys, .fragmentsAllowed]
    return try JSONSerialization.data(withJSONObject: left ?? [:], options: options)
        == JSONSerialization.data(withJSONObject: right ?? [:], options: options)
}
