import CodeIslandCore
import Foundation

// Rust runs this blocking C-ABI operation on its I/O pool. The Swift client
// owns credential parsing, HTTP policy, status interpretation and timestamps.
private final class FetchResult: @unchecked Sendable {
    let ready = DispatchSemaphore(value: 0)
    private var result: Result<ClaudeQuotaSnapshot, Error>?

    func finish(_ value: Result<ClaudeQuotaSnapshot, Error>) {
        result = value
        ready.signal()
    }

    func wait() -> Result<ClaudeQuotaSnapshot, Error> {
        ready.wait()
        return result!
    }
}

func fetchQuota(in directory: String) throws -> Data {
    let result = FetchResult()
    Task.detached {
        do {
            let snapshot = try await ClaudeQuotaClient.fetch(credential: {
                ClaudeCredentialStore.readFile(claudeHome: directory).flatMap(ClaudeCredentialStore.parse)
            })
            result.finish(.success(snapshot))
        } catch {
            result.finish(.failure(error))
        }
    }
    switch result.wait() {
    case .success(let snapshot):
        return try JSONEncoder().encode(Reply(snapshot))
    case .failure(let error):
        let reason: String
        var httpStatus: Int?
        switch error as? ClaudeQuotaClientError {
        case .noCredential: reason = "no_credential"
        case .unauthorized: reason = "unauthorized"
        case .rateLimited: reason = "rate_limited"
        case .http(let status): reason = "http"; httpStatus = status
        case .parse: reason = "parse"
        default: reason = "transport"
        }
        return try JSONEncoder().encode(Failure(error: reason, status: httpStatus))
    }
}

private struct Failure: Encodable { let error: String; let status: Int? }
private struct Reply: Encodable {
    let limits: [QuotaRow]
    let fetched_at: Int64
    init(_ snapshot: ClaudeQuotaSnapshot) {
        limits = snapshot.limits.map(QuotaRow.init)
        fetched_at = microseconds(snapshot.fetchedAt)
    }
}
