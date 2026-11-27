import CodeIslandCore
import Foundation

private struct RequestArguments: Decodable {
    let config: PushChannelConfig
    let kind: PushEventKind
    let title: String
    let headline: String
    let body: String
    let now: Double
}

private struct RenderedRequest: Encodable {
    let url: String
    let headers: [String: String]
    let body: String
}

private struct RequestResult: Encodable {
    var request: RenderedRequest?
    var problem: String?
}

func renderPush(_ data: Data) throws -> Data {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    let args = try decoder.decode(RequestArguments.self, from: data)
    let message = PushMessage(kind: args.kind, sessionId: "", title: args.title,
                              headline: args.headline, body: args.config.includeDetails ? args.body : "")
    do {
        let request = try PushRequestBuilder.request(for: message, channel: args.config,
                                                     now: Date(timeIntervalSince1970: args.now))
        return try JSONEncoder().encode(RequestResult(request: .init(url: request.url.absoluteString,
            headers: request.headers, body: String(decoding: request.body, as: UTF8.self))))
    } catch let problem as PushConfigProblem {
        return try JSONEncoder().encode(RequestResult(problem: problem.rawValue))
    }
}

private struct ResponseArguments: Decodable {
    let channel: PushChannelKind
    let status: Int
    let body: String
}

func acceptPush(_ data: Data) throws -> Data {
    let args = try JSONDecoder().decode(ResponseArguments.self, from: data)
    let result = PushResponseInterpreter.interpret(kind: args.channel, statusCode: args.status, body: Data(args.body.utf8))
    return try JSONEncoder().encode(result.ok)
}
