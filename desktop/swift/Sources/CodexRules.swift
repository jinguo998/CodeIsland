import CodeIslandCore
import Foundation

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
        switch args.home.map({ CodexPermissionRules.scope(for: event, codexHome: $0) }) ?? .session {
        case .session: scope = .init(kind: "session", patterns: nil)
        case .mcpTool: scope = .init(kind: "codex_mcp_config", patterns: nil)
        case .prefixes(let patterns): scope = .init(kind: "codex_rules", patterns: patterns)
        }
        return try encoder.encode(scope)
    default:
        guard let home = args.home, !home.isEmpty, CodexPermissionRules.isCodexEvent(event) else {
            return try encoder.encode(false)
        }
        let rules = CodexPermissionRules()
        if let patterns = args.patterns {
            return try encoder.encode(rules.persistPrefixRules(patterns, for: event, codexHome: home))
        }
        guard CodexPermissionRules.mcpToolApprovalTarget(for: event) != nil else { return try encoder.encode(false) }
        return try encoder.encode(rules.persistAlwaysAllowRule(for: event, codexHome: home))
    }
}
