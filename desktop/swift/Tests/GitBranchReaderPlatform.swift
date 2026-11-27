@testable import CodeIslandCore

// Run the original worktree and submodule fixtures through the same path
// resolution as the desktop app. The macOS package uses its own entry point.
extension GitBranchReaderTests {
    typealias GitBranchReader = DesktopGitBranchReader
}

extension DesktopGitBranchReader {
    static let parseHEAD = CodeIslandCore.GitBranchReader.parseHEAD
}
