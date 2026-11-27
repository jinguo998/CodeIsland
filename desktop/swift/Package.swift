// swift-tools-version: 5.9
import PackageDescription

// build-swift.py assembles this package in the build directory from the same
// source and test files as the macOS target, without resolving its UI packages.
let package = Package(
    name: "CodeIslandDesktopCore",
    products: [.library(name: "CodeIslandDesktopCore", type: .dynamic, targets: ["CodeIslandDesktopCore"])],
    targets: [
        .target(name: "CodeIslandCore"),
        .target(name: "CodeIslandDesktopCore", dependencies: ["CodeIslandCore"]),
        .testTarget(name: "CodeIslandCoreTests", dependencies: ["CodeIslandCore"]),
    ]
)
