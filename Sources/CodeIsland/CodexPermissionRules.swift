import CodeIslandCore
import Foundation

typealias CodexPermissionRules = CodeIslandCore.CodexPermissionRules

extension CodexPermissionRules {
    /// Registered macOS config roots are owned by the app's Settings.
    static func codexHome(for event: HookEvent) -> String {
        codexHome(for: event, roots: ConfigInstaller.codexHomes(), primary: ConfigInstaller.codexHome())
    }

    static func alwaysSavesRule(for event: HookEvent) -> Bool {
        alwaysSavesRule(for: event, codexHome: codexHome(for: event))
    }

    static func shouldDeferToCodexAutoReview(for event: HookEvent, fileManager: FileManager = .default) -> Bool {
        shouldDeferToCodexAutoReview(for: event, codexHome: codexHome(for: event), fileManager: fileManager)
    }

    @discardableResult
    func persistAlwaysAllowRule(for event: HookEvent) -> Bool {
        persistAlwaysAllowRule(for: event, codexHome: Self.codexHome(for: event))
    }

    @discardableResult
    func persistPrefixRules(_ patterns: [[String]], for event: HookEvent) -> Bool {
        persistPrefixRules(patterns, for: event, codexHome: Self.codexHome(for: event))
    }
}
