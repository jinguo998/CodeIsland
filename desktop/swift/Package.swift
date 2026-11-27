// swift-tools-version: 5.9
import PackageDescription
import Foundation

let sourceList = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("inputs.json")
struct TargetInputs: Decodable {
    let sources: [String]
    let exclude: [String]
}
let inputs = try JSONDecoder().decode([String: TargetInputs].self, from: Data(contentsOf: sourceList))

// Swift 6.2.3 emits a reference to a private FoundationCollections index
// descriptor for unoptimized AttributedString.runs on Windows (swift#88132).
// Optimized collection iteration uses the SDK's exported accessors.
let windowsAttributedString: [SwiftSetting] = [
    .unsafeFlags(["-O"], .when(platforms: [.windows], configuration: .debug)),
]

// inputs.json lists checkout references, desktop adapters and legacy generated files.
let package = Package(
    name: "CodeIslandDesktopCore",
    products: [.library(name: "CodeIslandDesktopCore", type: .dynamic, targets: ["CodeIslandDesktopCore"])],
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", exact: "3.15.1"),
        .package(url: "https://github.com/swiftlang/swift-cmark.git", exact: "0.6.0"),
    ],
    targets: [
        .systemLibrary(name: "SQLite3", path: "Targets/SQLite3"),
        .target(name: "CryptoKit", dependencies: [.product(name: "Crypto", package: "swift-crypto")], path: "Targets/CryptoKit"),
        .target(name: "CodeIslandCore", dependencies: [
            .target(name: "SQLite3", condition: .when(platforms: [.windows, .linux])), .target(name: "CryptoKit", condition: .when(platforms: [.windows, .linux])),
            .product(name: "cmark-gfm-extensions", package: "swift-cmark", condition: .when(platforms: [.windows, .linux])),
            .product(name: "cmark-gfm", package: "swift-cmark", condition: .when(platforms: [.windows, .linux])),
        ], path: "Targets/CodeIslandCore", exclude: inputs["CodeIslandCore"]!.exclude,
            sources: inputs["CodeIslandCore"]!.sources, swiftSettings: windowsAttributedString + [
            // The two upstream parsers import Darwin for C byte-search functions.
            .unsafeFlags(["-module-alias", "Darwin=Glibc"], .when(platforms: [.linux])),
            .unsafeFlags(["-module-alias", "Darwin=ucrt"], .when(platforms: [.windows])),
        ], linkerSettings: [
            .linkedLibrary("winsqlite3", .when(platforms: [.windows])),
            .linkedLibrary("sqlite3", .when(platforms: [.linux, .macOS])),
        ]),
        .target(name: "CodeIslandDesktopCore", dependencies: ["CodeIslandCore", .target(name: "SQLite3", condition: .when(platforms: [.windows, .linux]))],
            path: "Targets/CodeIslandDesktopCore", exclude: inputs["CodeIslandDesktopCore"]!.exclude,
            sources: inputs["CodeIslandDesktopCore"]!.sources),
        .testTarget(name: "CodeIslandCoreTests", dependencies: ["CodeIslandCore", "CodeIslandDesktopCore"],
            path: "Targets/CodeIslandCoreTests", exclude: inputs["CodeIslandCoreTests"]!.exclude,
            sources: inputs["CodeIslandCoreTests"]!.sources,
            swiftSettings: windowsAttributedString),
        .testTarget(name: "CodeIslandAppTests", dependencies: ["CodeIslandDesktopCore"],
            path: "Targets/CodeIslandAppTests", exclude: inputs["CodeIslandAppTests"]!.exclude,
            sources: inputs["CodeIslandAppTests"]!.sources,
            swiftSettings: [.unsafeFlags(["-module-alias", "CodeIsland=CodeIslandDesktopCore"])]),
    ]
)
