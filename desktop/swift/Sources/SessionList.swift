import CodeIslandCore
import Foundation

func nativeStatus(_ status: String, executing: Bool = false) -> AgentStatus {
    switch status {
    case "starting", "running": return executing ? .running : .processing
    case "waiting_permission": return .waitingApproval
    case "waiting_answer": return .waitingQuestion
    default: return .idle
    }
}

private struct Card: Decodable {
    let id: String
    let status: String
    let start: Int64
    let activity: Int64
    let parent: String?
}

private struct Queues: Decodable {
    let permissions: [String]
    let questions: [String]

    var ranks: [String: Int] {
        SessionListOrdering.requestRank(permissionSessionIds: permissions, questionSessionIds: questions)
    }
}

func sessionList(_ op: String, _ data: Data, _ existing: [String: SessionSnapshot]) throws -> Data {
    let decoder = JSONDecoder()
    let encoder = JSONEncoder()
    if op == "session.tier" {
        var snapshot = SessionSnapshot()
        snapshot.status = nativeStatus(try decoder.decode(String.self, from: data))
        return try encoder.encode(SessionCardStatus(snapshot).tier)
    }
    if op == "session.rank" { return try encoder.encode(decoder.decode(Queues.self, from: data).ranks) }
    struct Arguments: Decodable { let cards: [Card]; let queues: Queues }
    let args = try decoder.decode(Arguments.self, from: data)
    var sessions: [String: SessionSnapshot] = [:]
    for card in args.cards {
        var session = existing[card.id] ?? SessionSnapshot()
        session.status = nativeStatus(card.status)
        session.startTime = Date(timeIntervalSince1970: Double(card.start) / 1_000_000)
        session.lastActivity = Date(timeIntervalSince1970: Double(card.activity) / 1_000_000)
        session.parentSessionId = card.parent
        sessions[card.id] = session
    }
    if op == "session.order" {
        return try encoder.encode(SessionListOrdering.order(sessions, requestRank: args.queues.ranks))
    }
    let selection = IdleSessionRemovalPolicy.selection(sessions: sessions,
        blockedSessionIDs: Set(args.queues.permissions + args.queues.questions))
    return try encoder.encode(selection.groups.map { $0.sorted() })
}
