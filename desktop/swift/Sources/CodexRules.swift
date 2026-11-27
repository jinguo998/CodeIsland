import CodeIslandCore
import Foundation

// The runtime resolves Windows and WSL homes before entering the original rules engine.
enum ConfigInstaller {
    @TaskLocal static var home: String = ""
    static func codexHome() -> String { home }
    static func codexHomes() -> [String] { [home] }
}

func codexRule(_ operation: String, _ data: Data) throws -> Data {
    struct Arguments: Decodable {
        let home: String?
        let patterns: [[String]]?
    }
    struct Scope: Encodable {
        let kind: String
        let patterns: [[String]]?
    }
    let args = try JSONDecoder().decode(Arguments.self, from: data)
    let encoder = JSONEncoder()
    guard let event = try bridgeHook(data) else { return Data("null".utf8) }
    switch operation {
    case "codex.prefixes":
        return try encoder.encode(CodexPermissionRules.prefixPatterns(for: event))
    case "codex.matches":
        return try encoder.encode(CodexPermissionRules.commandsStart(event, withAnyOf: Set(args.patterns ?? [])))
    case "codex.scope":
        let scope: Scope
        switch args.home.flatMap({ $0.isEmpty ? nil : ConfigInstaller.$home.withValue($0) { ApprovalAlwaysScope(for: event) } }) ?? .session {
        case .session: scope = .init(kind: "session", patterns: nil)
        case .codexMCPConfig: scope = .init(kind: "codex_mcp_config", patterns: nil)
        case .codexRules(let patterns): scope = .init(kind: "codex_rules", patterns: patterns)
        }
        return try encoder.encode(scope)
    default:
        guard let home = args.home, !home.isEmpty, CodexPermissionRules.isCodexEvent(event) else {
            return try encoder.encode(false)
        }
        let rules = CodexPermissionRules()
        if let patterns = args.patterns {
            return try encoder.encode(ConfigInstaller.$home.withValue(home) { rules.persistPrefixRules(patterns, for: event) })
        }
        guard CodexPermissionRules.mcpToolApprovalTarget(for: event) != nil else { return try encoder.encode(false) }
        return try encoder.encode(ConfigInstaller.$home.withValue(home) { rules.persistAlwaysAllowRule(for: event) })
    }
}
