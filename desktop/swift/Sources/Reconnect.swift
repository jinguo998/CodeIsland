import CodeIslandCore
import Foundation

func reconnectOperation(_ op: String, _ data: Data, policy: inout RemoteReconnectPolicy) throws -> Data {
    struct Failure: Decodable { let id: String; let auto: Bool }
    struct Step: Encodable { let step: String; let seconds: Double? }
    let decoder = JSONDecoder()
    let encoder = JSONEncoder()
    let step: RemoteReconnectPolicy.Step
    switch op {
    case "ssh.start":
        step = policy.userConnected(try decoder.decode(String.self, from: data))
    case "ssh.connected":
        policy.tunnelConnected(try decoder.decode(String.self, from: data), now: Date())
        return Data("null".utf8)
    case "ssh.stop":
        policy.userDisconnected(try decoder.decode(String.self, from: data))
        return Data("null".utf8)
    case "ssh.failed":
        let failure = try decoder.decode(Failure.self, from: data)
        step = policy.tunnelFailed(failure.id, autoRetry: failure.auto, now: Date())
    case "ssh.due":
        step = policy.retryDue(try decoder.decode(String.self, from: data))
    case "ssh.recover":
        let id = try decoder.decode(String.self, from: data)
        step = policy.recover()[id] ?? .none
    case "ssh.wake":
        policy.systemWoke()
        return try encoder.encode(policy.config.wakeSettle)
    case "ssh.network":
        guard let online = try decoder.decode(Bool?.self, from: data) else {
            policy.networkUnknown()
            return Data("null".utf8)
        }
        return try encoder.encode(policy.networkChanged(satisfied: online) ? policy.config.networkSettle : nil)
    default:
        throw DecodingError.dataCorrupted(.init(codingPath: [], debugDescription: "Unknown reconnect event"))
    }
    switch step {
    case .connect: return try encoder.encode(Step(step: "connect", seconds: nil))
    case .retry(let delay, _), .retrySlowly(let delay, _): return try encoder.encode(Step(step: "retry", seconds: delay))
    case .waitForNetwork(let delay): return try encoder.encode(Step(step: "offline", seconds: delay))
    case .none: return try encoder.encode(Step(step: "none", seconds: nil))
    }
}
