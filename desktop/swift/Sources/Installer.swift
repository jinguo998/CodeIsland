import CodeIslandCore
import Foundation

func installerOperation(_ operation: String, _ data: Data) throws -> Data {
    let encoder = JSONEncoder()
    let decoder = JSONDecoder()
    func eventList(_ entries: [(String, Int, Bool)], source: String) -> [[String: Any]] {
        entries.map { (name, timeout, _) in
            ["name": name, "timeout_ms": ["gemini", "qwen"].contains(source) ? timeout : timeout * 1000,
             "blocking": EventNormalizer.normalize(name) == "PermissionRequest"]
        }
    }
    switch operation {
    case "install.remoteScript":
        let template = try decoder.decode(String.self, from: data)
        return try encoder.encode(template.replacingOccurrences(of: "# @codeisland-swift-remote-helpers@",
            with: DesktopRemoteInstaller.helpers))
    case "install.shellWord":
        return try encoder.encode(DesktopRemoteInstaller.shellWord(decoder.decode(String.self, from: data)))
    case "install.remotePlugin":
        struct Args: Decodable { let source: String; let id: String; let name: String; let socket: String; let hook: String }
        let args = try decoder.decode(Args.self, from: data)
        let host = RemoteHost(id: args.id, name: args.name, host: "")
        let source = DesktopRemoteInstaller.remoteOpencodePluginForInstall(source: args.source, host: host, remoteSocketPath: args.socket)
        // The Windows tunnel owns a per-host hook directory on the remote machine.
        let original = #"const HOOK_PATH = `${process.env.HOME}/.codeisland/codeisland-remote-hook.py`;"#
        let configured = "const HOOK_PATH = " + DesktopRemoteInstaller.quotedLiteral(args.hook) + ";"
        return try encoder.encode(source.replacingOccurrences(of: original, with: configured))
    case "install.events":
        let entries = Dictionary(uniqueKeysWithValues: ConfigInstaller.desktopCLIs.map {
            ($0.source, eventList($0.events, source: $0.source))
        })
        return try JSONSerialization.data(withJSONObject: entries)
    case "install.defaults":
        let format = try decoder.decode(String.self, from: data)
        let rows = HookFormat(storageValue: format).map { ConfigInstaller.defaultEvents(for: $0) } ?? []
        return try JSONSerialization.data(withJSONObject: eventList(rows, source: ""))
    case "install.mimoPlugin":
        return try encoder.encode(ConfigInstaller.mimoPluginSource(from: decoder.decode(String.self, from: data)))
    case "install.minimaxManifest":
        return try JSONSerialization.data(withJSONObject: ConfigInstaller.minimaxManifestDocument(), options: .sortedKeys)
    case "install.jsonEdit":
        struct Edits: Decodable { let original: String; let remove: [String] }
        let edits = try decoder.decode(Edits.self, from: data)
        let args = try JSONSerialization.jsonObject(with: data) as? [String: Any]
        var text = edits.original
        for key in edits.remove {
            guard let next = JSONMinimalEditor.deleteTopLevelKey(in: text, key: key) else { return Data("null".utf8) }
            text = next
        }
        let changed = args?["set"] as? [String: Any] ?? [:]
        for key in changed.keys.sorted() {
            guard let next = JSONMinimalEditor.setTopLevelValue(in: text, key: key, value: changed[key]!) else { return Data("null".utf8) }
            text = next
        }
        return try encoder.encode(text)
    case "install.opencode":
        struct Plugin: Decodable { let original: String; let reference: String? }
        let args = try decoder.decode(Plugin.self, from: data)
        let id = "codeisland.js"
        if let ref = args.reference {
            return try encoder.encode(ConfigInstaller.mergeOpencodePluginRef(originalContents: args.original,
                pluginRef: ref, identifier: id))
        }
        return try encoder.encode(ConfigInstaller.removeOpencodePluginRef(originalContents: args.original, identifier: id) ?? args.original)
    case "install.zcodeEntry":
        struct Entry: Decodable { let command: String; let timeout: Int }
        let entry = try decoder.decode(Entry.self, from: data)
        return try JSONSerialization.data(withJSONObject: ConfigInstaller.zcodeEntry(command: entry.command, timeout: entry.timeout), options: .sortedKeys)
    case "install.kimiBlock", "install.traeCommand":
        struct Entry: Decodable { let event: String; let timeout: Int; let command: String }
        let entry = try decoder.decode(Entry.self, from: data)
        if operation == "install.traeCommand" {
            return try encoder.encode(ConfigInstaller.traeCommand(event: entry.event, timeout: entry.timeout, command: entry.command))
        }
        return try encoder.encode(ConfigInstaller.kimiBlock(event: entry.event, timeout: entry.timeout, baseCommand: entry.command))
    case "install.stripComments":
        return try encoder.encode(ConfigInstaller.stripJSONComments(decoder.decode(String.self, from: data)))
    case "install.claudeRoot":
        struct Roots: Decodable { let home: String; let preference: String?; let environment: String? }
        let roots = try decoder.decode(Roots.self, from: data)
        let dir = ClaudeConfigPaths.resolve(preference: roots.preference, environment: roots.environment,
            homeDir: roots.home) { path in
                var isDirectory: ObjCBool = false
                return FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory) && isDirectory.boolValue
            }
        return try encoder.encode(dir)
    case "install.entry":
        struct Args: Decodable { let format: String; let source: String; let event: String; let timeout: Int; let command: String }
        let args = try decoder.decode(Args.self, from: data)
        guard let format = HookFormat(storageValue: args.format) else { return Data("null".utf8) }
        let cli = CLIConfig(name: args.source, source: args.source, configPath: "", configKey: "hooks", format: format, events: [])
        // The executable expects --stdin plus an explicit event on Windows.
        // Fill the command fields of the newly built entry, never a user's existing entries.
        func commands(_ value: Any) -> Any {
            if let object = value as? [String: Any] {
                return object.mapValues(commands)
            }
            if let array = value as? [Any] { return array.map(commands) }
            if let value = value as? String, value.hasPrefix("@desktop-hook@") { return args.command }
            return value
        }
        var entry: Any?
        _ = ConfigInstaller.writeEntry(cli: cli, event: args.event, timeout: args.timeout, baseCommand: "@desktop-hook@") {
            entry = commands($0)
        }
        return try JSONSerialization.data(withJSONObject: entry ?? NSNull(), options: [.fragmentsAllowed, .sortedKeys])
    default: return Data("null".utf8)
    }
}
