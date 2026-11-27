import CodeIslandCore
import Foundation
import SQLite3

/// Threads recorded by the Codex Desktop app-server in its read-only state store.
public enum CodexStateStore {
    public struct Row: Encodable {
        public let id: String
        public let path: String
        public let cwd: String?
        public let modified: Double
        public let created: Double?
        public let source: String
        public let originator: String?
        public let model: String?
        public let reasoning: String?
        public let title: String?
        public let prompt: String?
        public let branch: String?
        public let parent: String?
        public let edgeStatus: String?
    }

    public static func recentRows(at statePath: String, now: Date, freshnessWindow: TimeInterval = 600) -> [Row]? {
        // `has_user_event` is not a reliable visibility bit for Desktop threads:
        // current ChatGPT/Codex builds may keep it at 0 while a user turn is
        // actively writing to the rollout. Pull a slightly wider DB candidate
        // window, then enforce the tighter freshness window from DB/file mtimes.
        let candidateWindow = max(freshnessWindow, 60 * 60)
        let cutoff = now.addingTimeInterval(-candidateWindow).timeIntervalSince1970

        return withProviderDatabase(at: statePath) { db in
            let columns = SQLiteReader.sqliteTableColumns(db: db, tableName: "threads")
            let requiredColumns: Set<String> = [
                "id", "rollout_path", "cwd", "archived", "source",
            ]
            guard requiredColumns.isSubset(of: columns) else { return [] }

            func timestamp(_ column: String) -> String? {
                let seconds = columns.contains(column)
                let millis = "\(column)_ms"
                if columns.contains(millis) {
                    let value = "thread.\(millis)"
                    return seconds ? "COALESCE(NULLIF(\(value), 0) / 1000.0, thread.\(column))" : "\(value) / 1000.0"
                }
                return seconds ? "thread.\(column)" : nil
            }
            guard let updated = timestamp("updated_at") else { return [] }
            func optional(_ column: String) -> String { columns.contains(column) ? "thread.\(column)" : "NULL" }
            let spawnColumns = SQLiteReader.sqliteTableColumns(db: db, tableName: "thread_spawn_edges")
            let hasParent = Set(["child_thread_id", "parent_thread_id"]).isSubset(of: spawnColumns)
            let hasStatus = Set(["child_thread_id", "status"]).isSubset(of: spawnColumns)
            let spawnJoin = hasParent || hasStatus
                ? "LEFT JOIN thread_spawn_edges edge ON edge.child_thread_id = thread.id" : ""

            guard let statement = SQLiteReader.prepareSQLiteStatement(db: db, sql: """
                SELECT thread.id, thread.rollout_path, thread.cwd, \(updated),
                       \(timestamp("created_at") ?? "NULL"), thread.source, \(optional("originator")),
                       \(optional("model")), \(optional("reasoning_effort")),
                       COALESCE(NULLIF(\(optional("name")), ''), \(optional("title"))),
                       COALESCE(NULLIF(\(optional("preview")), ''), \(optional("first_user_message"))),
                       \(optional("git_branch")), \(hasParent ? "edge.parent_thread_id" : "NULL"),
                       \(hasStatus ? "edge.status" : "NULL")
                FROM threads AS thread \(spawnJoin)
                WHERE thread.archived = 0 AND \(updated) >= ?
                  AND (LOWER(thread.source) IN ('vscode', 'appserver', 'app_server')
                       OR thread.source LIKE '{"subagent%')
                ORDER BY \(updated) DESC LIMIT 50;
                """) else { return [] }
            defer { sqlite3_finalize(statement) }
            sqlite3_bind_double(statement, 1, cutoff)

            var rows: [Row] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                func text(_ index: Int32) -> String? {
                    guard let value = SQLiteReader.sqliteColumnString(statement, index: index),
                          !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
                    return value
                }
                guard let id = text(0), let path = text(1), let source = text(5) else { continue }
                let modified = sqlite3_column_double(statement, 3)
                guard modified.isFinite, modified > 0 else { continue }
                let created = sqlite3_column_double(statement, 4)
                rows.append(Row(id: id, path: path, cwd: text(2), modified: modified,
                                created: created.isFinite && created > 0 ? created : nil,
                                source: source, originator: text(6), model: text(7), reasoning: text(8),
                                title: text(9), prompt: text(10), branch: text(11), parent: text(12),
                                edgeStatus: text(13)))
            }
            return rows
        }
    }

    public static func visibleStatus(modifiedAt: Date, now: Date, turnStatus: ConversationTurnStatus?,
                                     freshnessWindow: TimeInterval = 600, completionSettleWindow: TimeInterval = 30) -> AgentStatus? {
        guard modifiedAt >= now.addingTimeInterval(-freshnessWindow) else { return nil }
        let age = now.timeIntervalSince(modifiedAt)
        if age <= 300 && turnStatus == .processing { return .processing }
        return age <= completionSettleWindow ? .idle : nil
    }
}

// SQLite on writable Windows/WSL mounts may create empty -wal/-shm files
// even for a read-only connection. A checkpointed WAL database can be read
// without touching the provider's directory. Live writers use the native reader.
private func withProviderDatabase<T>(at path: String, _ body: (OpaquePointer) -> T?) -> T? {
    let fm = FileManager.default
    func hasSidecars() -> Bool { fm.fileExists(atPath: path + "-wal") || fm.fileExists(atPath: path + "-shm") }
    guard !hasSidecars(), let handle = FileHandle(forReadingAtPath: path) else {
        return SQLiteReader.withSQLiteDatabase(at: path, body: body)
    }
    let header = try? handle.read(upToCount: 20)
    try? handle.close()
    guard let header, header.count == 20, header[18] == 2, header[19] == 2 else {
        return SQLiteReader.withSQLiteDatabase(at: path, body: body)
    }
    var components = URLComponents(url: URL(fileURLWithPath: path), resolvingAgainstBaseURL: false)
    components?.queryItems = [URLQueryItem(name: "immutable", value: "1")]
    guard let uri = components?.string else { return nil }
    var database: OpaquePointer?
    let status = sqlite3_open_v2(uri, &database, SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX | SQLITE_OPEN_URI, nil)
    defer { if let database { sqlite3_close_v2(database) } }
    guard status == SQLITE_OK, let database, !hasSidecars() else {
        return SQLiteReader.withSQLiteDatabase(at: path, body: body)
    }
    let result = body(database)
    return hasSidecars() ? SQLiteReader.withSQLiteDatabase(at: path, body: body) : result
}
