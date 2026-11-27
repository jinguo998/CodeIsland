import Foundation

public enum CodexTranscriptOrigin: String, Equatable, Sendable {
    case desktop
    case cli
    case unknown
}

public enum CodexRollout {
    public static func origin(originator: String?, source: String?, isSubagentSource: Bool) -> CodexTranscriptOrigin {
        let originator = originator?.lowercased() ?? ""
        let source = source?.lowercased() ?? ""
        if originator.contains("desktop") {
            return .desktop
        }
        // Codex records every app-server client under `vscode`/`appServer`,
        // and a spawned thread keeps its parent's originator under an object
        // source. A client that names itself something other than Desktop
        // (Claude Code's Codex plugin sends "Claude Code") runs its own
        // `codex app-server`, so the rollout belongs to that process like a
        // CLI one.
        let isAppServerSource = source == "vscode" || source == "appserver" || source == "app_server"
        if !originator.isEmpty, isAppServerSource || isSubagentSource {
            return .cli
        }
        if isAppServerSource {
            return .desktop
        }
        if originator.contains("cli") || source == "cli" || source == "exec" {
            return .cli
        }
        return .unknown
    }

    public static func subagent(in payload: [String: Any]) -> CodexTranscriptSubagentInspection {
        guard let source = payload["source"] as? [String: Any] else {
            return .root
        }
        let threadId = (payload["id"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        let payloadParent = (payload["parent_thread_id"] as? String).flatMap { $0.isEmpty ? nil : $0 }
        // Unit variants (`review`, `compact`, `memory_consolidation`) serialize as
        // a bare string and only record their parent on the payload. Without
        // one there is no spawn edge to fall back to either.
        if let label = source["subagent"] as? String, !label.isEmpty {
            guard let payloadParent else { return .root }
            return .subagent(CodexSubagentMetadata(
                parentThreadId: payloadParent,
                agentType: label,
                agentNickname: nil,
                threadId: threadId
            ))
        }
        guard let subagent = source["subagent"] as? [String: Any],
              !subagent.isEmpty else {
            return .root
        }
        // Spawned workers nest the parent under `source.subagent.thread_spawn`;
        // auto-review (guardian) threads record it on the payload itself.
        let parent = firstStringRecursively(in: subagent, key: "parent_thread_id")
            ?? payloadParent
        guard let parent, !parent.isEmpty else { return .unavailable }

        let agentType = firstStringRecursively(in: subagent, key: "agent_role")
            ?? subagent["other"] as? String
            ?? subagent.keys.sorted().first
        let nickname = firstStringRecursively(in: subagent, key: "agent_nickname")
        return .subagent(CodexSubagentMetadata(
            parentThreadId: parent,
            agentType: agentType,
            agentNickname: nickname,
            threadId: threadId
        ))
    }

    public static func firstStringRecursively(in value: Any, key: String) -> String? {
        if let dict = value as? [String: Any] {
            if let string = dict[key] as? String,
               !string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                return string
            }
            for child in dict.values {
                if let found = firstStringRecursively(in: child, key: key) {
                    return found
                }
            }
        } else if let array = value as? [Any] {
            for child in array {
                if let found = firstStringRecursively(in: child, key: key) {
                    return found
                }
            }
        }
        return nil
    }

    /// Scan a rollout backwards in chunks until the latest lifecycle marker is
    /// found. This avoids both a full-file read and a fixed-tail limit while
    /// reusing `JSONLTailer`'s canonical Codex event interpretation.
    public static func latestTurnStatus(
        path: String,
        chunkSize: UInt64 = 256 * 1024,
        maxBytesToScan: UInt64 = 64 * 1024 * 1024,
        maxLineBytes: Int = 4 * 1024 * 1024,
        requireFinalNewline: Bool = false
    ) -> ConversationTurnStatus? {
        guard chunkSize > 0, maxBytesToScan > 0, maxLineBytes > 0 else { return nil }
        guard let handle = FileHandle(forReadingAtPath: path) else { return nil }
        defer { handle.closeFile() }

        let fileSize = handle.seekToEndOfFile()
        let lowerBound = fileSize > maxBytesToScan ? fileSize - maxBytesToScan : 0
        var cursor = fileSize
        var leadingFragment = Data()
        var discardingOversizedLine = false

        // A live tailer commits only newline-terminated rows; a closed rollout
        // may also end with a complete JSON value without a final newline.
        var discardFinalFragment = requireFinalNewline
        while cursor > lowerBound {
            let readSize = min(cursor - lowerBound, chunkSize)
            cursor -= readSize
            handle.seek(toFileOffset: cursor)
            var chunk = handle.readData(ofLength: Int(readSize))
            if discardFinalFragment {
                guard let newline = chunk.lastIndex(of: 0x0A) else { continue }
                chunk = Data(chunk[...newline])
                discardFinalFragment = false
            }
            var combined: Data
            if discardingOversizedLine {
                // We are walking backwards through one over-budget line. Drop
                // its bytes until the previous newline, then resume with older
                // complete lines instead of retaining/copying an unbounded blob.
                guard let newline = chunk.lastIndex(of: 0x0A) else { continue }
                combined = Data(chunk[...newline])
                discardingOversizedLine = false
            } else {
                combined = chunk
                combined.append(leadingFragment)
            }

            var lines: [Data] = []
            var lineStart = combined.startIndex
            for index in combined.indices where combined[index] == 0x0A {
                lines.append(Data(combined[lineStart..<index]))
                lineStart = combined.index(after: index)
            }
            lines.append(Data(combined[lineStart..<combined.endIndex]))
            if cursor > lowerBound, !lines.isEmpty {
                let fragment = lines.removeFirst()
                if fragment.count > maxLineBytes {
                    leadingFragment.removeAll(keepingCapacity: true)
                    discardingOversizedLine = true
                } else {
                    leadingFragment = fragment
                }
            } else {
                leadingFragment.removeAll(keepingCapacity: true)
                // A capped scan begins in the middle of an unknown line. Never
                // parse that boundary fragment as if it were complete JSON.
                if lowerBound > 0, !lines.isEmpty {
                    lines.removeFirst()
                }
            }

            for line in lines.reversed()
            where !line.isEmpty && line.count <= maxLineBytes {
                if let status = codexTurnStatus(fromCompleteLine: line) {
                    return status
                }
            }
        }

        return nil
    }

    private static func codexTurnStatus(fromCompleteLine line: Data) -> ConversationTurnStatus? {
        var newlineTerminated = line
        newlineTerminated.append(0x0A)
        return JSONLTailer.latestTurnStatus(in: newlineTerminated)
    }
}
