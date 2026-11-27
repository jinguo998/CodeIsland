// swift-tools-version: 5.9
import PackageDescription

// build-swift.py assembles this package in the build directory from the same
// source and test files as the macOS target, without resolving its UI packages.
let package = Package(
    name: "CodeIslandDesktopCore",
    products: [.library(name: "CodeIslandDesktopCore", type: .dynamic, targets: ["CodeIslandDesktopCore"])],
    dependencies: [.package(url: "https://github.com/apple/swift-crypto.git", exact: "3.15.1")],
    targets: [
        .systemLibrary(name: "SQLite3"),
        .target(name: "CryptoKit", dependencies: [.product(name: "Crypto", package: "swift-crypto")]),
        .target(name: "CodeIslandCore", dependencies: [
            .target(name: "SQLite3", condition: .when(platforms: [.windows, .linux])), .target(name: "CryptoKit", condition: .when(platforms: [.windows, .linux])),
        ], linkerSettings: [
            .linkedLibrary("winsqlite3", .when(platforms: [.windows])),
            .linkedLibrary("sqlite3", .when(platforms: [.linux, .macOS])),
        ]),
        .target(name: "CodeIslandDesktopCore", dependencies: ["CodeIslandCore", .target(name: "SQLite3", condition: .when(platforms: [.windows, .linux]))]),
        .testTarget(name: "CodeIslandCoreTests", dependencies: ["CodeIslandCore"]),
    ]
)
