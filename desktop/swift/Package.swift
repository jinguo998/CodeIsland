// swift-tools-version: 5.9
import PackageDescription

// Swift 6.2.3 emits a reference to a private FoundationCollections index
// descriptor for unoptimized AttributedString.runs on Windows (swift#88132).
// Optimized collection iteration uses the SDK's exported accessors.
let windowsAttributedString: [SwiftSetting] = [
    .unsafeFlags(["-O"], .when(platforms: [.windows], configuration: .debug)),
]

// build-swift.py assembles this package in the build directory from the same
// source and test files as the macOS target, without resolving its UI packages.
let package = Package(
    name: "CodeIslandDesktopCore",
    products: [.library(name: "CodeIslandDesktopCore", type: .dynamic, targets: ["CodeIslandDesktopCore"])],
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", exact: "3.15.1"),
        .package(url: "https://github.com/swiftlang/swift-cmark.git", exact: "0.6.0"),
    ],
    targets: [
        .systemLibrary(name: "SQLite3"),
        .target(name: "CryptoKit", dependencies: [.product(name: "Crypto", package: "swift-crypto")]),
        .target(name: "CodeIslandCore", dependencies: [
            .target(name: "SQLite3", condition: .when(platforms: [.windows, .linux])), .target(name: "CryptoKit", condition: .when(platforms: [.windows, .linux])),
            .product(name: "cmark-gfm-extensions", package: "swift-cmark", condition: .when(platforms: [.windows, .linux])),
            .product(name: "cmark-gfm", package: "swift-cmark", condition: .when(platforms: [.windows, .linux])),
        ], swiftSettings: windowsAttributedString, linkerSettings: [
            .linkedLibrary("winsqlite3", .when(platforms: [.windows])),
            .linkedLibrary("sqlite3", .when(platforms: [.linux, .macOS])),
        ]),
        .target(name: "CodeIslandDesktopCore", dependencies: ["CodeIslandCore", .target(name: "SQLite3", condition: .when(platforms: [.windows, .linux]))]),
        .testTarget(name: "CodeIslandCoreTests", dependencies: ["CodeIslandCore", "CodeIslandDesktopCore"],
            swiftSettings: windowsAttributedString),
    ]
)
