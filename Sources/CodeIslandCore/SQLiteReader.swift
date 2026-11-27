import Foundation
#if canImport(SQLite3)
import SQLite3
#else
import DesktopSQLite
#endif

/// Read-only access to the SQLite session stores discovery reads: Codex's
/// state database and OpenCode's (and MiMo Code's) store.
public enum SQLiteReader {
    public static func withSQLiteDatabase<T>(
        at path: String,
        body: (OpaquePointer) -> T?
    ) -> T? {
        guard let db = openReadOnly(path) else { return nil }
        sqlite3_busy_timeout(db, 1000)
        defer { sqlite3_close_v2(db) }
        return body(db)
    }

    /// A read-only connection to the store at `path`.
    ///
    /// A WAL database whose -wal and -shm files are gone can't be read by a
    /// read-only connection: it may not create them, so every statement fails
    /// with SQLITE_CANTOPEN. Codex leaves `state_5.sqlite` that way whenever
    /// its last connection closes (an idle Codex Desktop). With no writer on
    /// it, the file holds the whole database, so it is then opened
    /// `immutable`, which reads it without creating, locking or changing
    /// anything. Never read-write: closing the last read-write connection
    /// would checkpoint into the agent's own database.
    private static func openReadOnly(_ path: String) -> OpaquePointer? {
        // Some SQLite builds can create the WAL and shared-memory files from
        // a read-only connection when the directory allows writes. Open an
        // already checkpointed WAL database without creating either file.
        if isClosedWAL(path), let db = openImmutable(path) {
            if !hasWALFiles(path) { return db }
            sqlite3_close_v2(db)
        }
        guard let db = open(path, flags: SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX) else { return nil }
        // Opening is lazy: the first statement is where the WAL index is needed.
        guard sqlite3_exec(db, "SELECT 1 FROM sqlite_master LIMIT 1;", nil, nil, nil) == SQLITE_CANTOPEN else {
            return db
        }
        sqlite3_close_v2(db)
        return isClosedWAL(path) ? openImmutable(path) : nil
    }

    private static func hasWALFiles(_ path: String) -> Bool {
        FileManager.default.fileExists(atPath: path + "-wal") || FileManager.default.fileExists(atPath: path + "-shm")
    }

    private static func isClosedWAL(_ path: String) -> Bool {
        guard !hasWALFiles(path), let file = FileHandle(forReadingAtPath: path) else { return false }
        defer { try? file.close() }
        guard let header = try? file.read(upToCount: 20), header.count == 20 else { return false }
        return header[18] == 2 && header[19] == 2
    }

    private static func openImmutable(_ path: String) -> OpaquePointer? {
        guard var components = URLComponents(url: URL(fileURLWithPath: path), resolvingAgainstBaseURL: false) else { return nil }
        components.queryItems = [URLQueryItem(name: "immutable", value: "1")]
        guard let uri = components.string else { return nil }
        return open(uri, flags: SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX | SQLITE_OPEN_URI)
    }

    private static func open(_ filename: String, flags: Int32) -> OpaquePointer? {
        var db: OpaquePointer?
        guard sqlite3_open_v2(filename, &db, flags, nil) == SQLITE_OK, let db else {
            if let db { sqlite3_close_v2(db) }
            return nil
        }
        return db
    }

    public static func prepareSQLiteStatement(db: OpaquePointer, sql: String) -> OpaquePointer? {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &statement, nil) == SQLITE_OK else {
            if let statement { sqlite3_finalize(statement) }
            return nil
        }
        return statement
    }

    public static func sqliteTableColumns(
        db: OpaquePointer,
        tableName: String
    ) -> Set<String> {
        guard tableName.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "_" }),
              let statement = prepareSQLiteStatement(
                db: db,
                sql: "PRAGMA table_info(\(tableName));"
              ) else {
            return []
        }
        defer { sqlite3_finalize(statement) }

        var columns = Set<String>()
        while sqlite3_step(statement) == SQLITE_ROW {
            if let name = sqliteColumnString(statement, index: 1) {
                columns.insert(name)
            }
        }
        return columns
    }

    public static func bindSQLiteText(_ text: String, to statement: OpaquePointer, index: Int32) {
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        _ = text.withCString { pointer in
            sqlite3_bind_text(statement, index, pointer, -1, transient)
        }
    }

    public static func sqliteColumnString(_ statement: OpaquePointer, index: Int32) -> String? {
        guard let value = sqlite3_column_text(statement, index) else { return nil }
        return String(cString: UnsafeRawPointer(value).assumingMemoryBound(to: CChar.self))
    }

    public static func nonEmptySQLiteColumnString(_ statement: OpaquePointer, index: Int32) -> String? {
        guard let value = sqliteColumnString(statement, index: index)?
            .trimmingCharacters(in: .whitespacesAndNewlines),
              !value.isEmpty else {
            return nil
        }
        return value
    }
}
