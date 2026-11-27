import Foundation

#if os(Windows)
public typealias pid_t = Int32

// ConfigPathIdentity owns (and frees) realpath's result on the macOS ABI.
func realpath(_ path: String, _ unused: UnsafeMutablePointer<CChar>?) -> UnsafeMutablePointer<CChar>? {
    guard FileManager.default.fileExists(atPath: path) else { return nil }
    let resolved = URL(fileURLWithPath: path).resolvingSymlinksInPath().path
    let bytes = resolved.utf8CString
    guard let destination = malloc(bytes.count) else { return nil }
    bytes.withUnsafeBufferPointer { buffer in
        destination.copyMemory(from: buffer.baseAddress!, byteCount: buffer.count)
    }
    return destination.assumingMemoryBound(to: CChar.self)
}
#endif

// Darwin's byte search is used by the original AgentTaskTranscript fast path.
func memmem(_ bytes: UnsafeRawPointer, _ length: Int, _ marker: UnsafeRawPointer?, _ count: Int) -> UnsafeRawPointer? {
    guard let marker, count > 0, length >= count else { return nil }
    let first = Int32(marker.load(as: UInt8.self))
    var offset = 0
    while offset <= length - count {
        guard let ptr = memchr(bytes + offset, first, length - count - offset + 1) else { return nil }
        let match = UnsafeRawPointer(ptr)
        if memcmp(match, marker, count) == 0 { return match }
        offset = bytes.distance(to: match) + 1
    }
    return nil
}

extension ClaudeCredentialStore {
    public static func load() -> ClaudeOAuthCredential? { readFile().flatMap(parse) }
}

extension ClaudeUsageScanner.FileCache {
    public var hasScannedFiles: Bool { !files.isEmpty }
}

extension URLSessionPushTransport {
    public static func desktopRetryAfter(_ value: String?) -> TimeInterval? {
        retryAfter(value).flatMap { $0.isFinite ? $0 : nil }
    }
}

extension FollowUpReminderKind: Codable {}
extension FollowUpReminder.Origin: Codable {}
extension FollowUpReminder.Delivery: Codable {}

#if !canImport(Darwin)
func CFNumberIsFloatType(_ value: NSNumber) -> Bool {
    ["f", "d"].contains(String(cString: value.objCType))
}
#endif

#if canImport(FoundationNetworking)
import FoundationNetworking

// swift-corelibs installs redirect delegates on the session. Retain the
// upstream ephemeral configuration, including its cookie and cache policy.
private final class DesktopQuotaRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}
extension ClaudeQuotaClient {
    public static let desktopSession = URLSession(configuration: session.configuration,
                                                  delegate: DesktopQuotaRedirects(), delegateQueue: nil)
}
#else
extension ClaudeQuotaClient { public static var desktopSession: URLSession { session } }
#endif

public enum DesktopPath {
    public static func isWindowsAbsolute(_ path: String) -> Bool {
        if path.hasPrefix(#"\\"#) { return true }
        let start = Array(path.utf8.prefix(3))
        return start.count == 3 && ((65...90).contains(start[0]) || (97...122).contains(start[0]))
            && start[1] == 58 && (start[2] == 47 || start[2] == 92)
    }

    public static func withForwardSlashes(_ path: String) -> String {
        isWindowsAbsolute(path) ? path.replacingOccurrences(of: #"\"#, with: "/") : path
    }
}

extension GitBranchReader {
    public static func desktopBranch(cwd: String) -> String? {
        let start = DesktopPath.withForwardSlashes(cwd)
        if let branch = read(cwd: start)?.branch { return branch }
        // The Windows workspace can be deeper than the native 12-directory search;
        // a worktree pointer may also start with a drive letter or a UNC share.
        var directory = URL(fileURLWithPath: start, isDirectory: true)
        for _ in 0..<64 {
            let git = directory.appendingPathComponent(".git")
            var isDirectory: ObjCBool = false
            if FileManager.default.fileExists(atPath: git.path, isDirectory: &isDirectory) {
                if isDirectory.boolValue { return headInfo(gitDir: git.path, isWorktree: false)?.branch }
                guard let pointer = try? String(contentsOf: git, encoding: .utf8) else { return nil }
                #if os(Windows)
                let line = pointer.trimmingCharacters(in: .whitespacesAndNewlines)
                if line.hasPrefix("gitdir:") {
                    let target = String(line.dropFirst("gitdir:".count)).trimmingCharacters(in: .whitespaces)
                    if DesktopPath.isWindowsAbsolute(target) {
                        return headInfo(gitDir: target, isWorktree: false)?.branch
                    }
                }
                #endif
                guard let target = parseGitdirPointer(pointer, relativeTo: directory.path) else { return nil }
                return headInfo(gitDir: target, isWorktree: false)?.branch
            }
            let parent = directory.deletingLastPathComponent()
            if directory.path == parent.path { break }
            directory = parent
        }
        return nil
    }
}
