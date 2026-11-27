import CodeIslandCore
import Foundation

func codexMessage(_ operation: String, _ data: Data) throws -> Data {
    if operation == "codex.questionResult" {
        let answers = try JSONDecoder().decode([String: [String]].self, from: data)
        return try JSONSerialization.data(withJSONObject: CodexAppServerProtocol.questionResult(answersByKey: answers))
    }
    guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return Data((operation == "codex.questions" ? "[]" : "null").utf8)
    }
    switch operation {
    case "codex.rpc":
        guard let message = CodexAppServerProtocol.parseMessage(obj) else { return Data("null".utf8) }
        var result: [String: Any] = ["params": obj["params"] ?? [:], "has_result": false]
        switch message.kind {
        case .request(let method, let id): result["method"] = method; result["id"] = id.jsonValue
        case .notification(let method): result["method"] = method
        case .response(let id): result["id"] = id.jsonValue; result["has_result"] = obj["result"] != nil
        case .error(let id, _, _): result["id"] = id?.jsonValue
        }
        return try JSONSerialization.data(withJSONObject: result)
    case "codex.questions":
        let items = CodexAppServerProtocol.questions(in: AnyCodableLike.from(obj).asObject ?? [:])
        let rows: [[String: Any]] = items.map { item in
            var row: [String: Any] = ["question": item.payload.question, "answer_key": item.answerKey,
                                       "isSecret": item.payload.isSecret]
            row["header"] = item.payload.header
            row["options"] = item.payload.options?.enumerated().map { index, label in
                ["label": label, "description": item.payload.descriptions?[index] ?? ""]
            }
            return row
        }
        return try JSONSerialization.data(withJSONObject: rows)
    default:
        var session = SessionSnapshot()
        guard CodexAppServerProtocol.applyThreadStatus(&session, status: AnyCodableLike.from(obj).asObject) else {
            return Data("null".utf8)
        }
        let status: String
        switch session.status {
        case .waitingApproval: status = "waiting_permission"
        case .waitingQuestion: status = "waiting_answer"
        case .idle: status = "idle"
        case .running, .processing: status = "running"
        }
        return try JSONEncoder().encode(status)
    }
}
