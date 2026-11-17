// swift-tools-version: 6.0
// The Mac app: SwiftUI and AppKit around the Rust engine (../engine), which scripts/build-app.sh builds into
// build/lib/libinterleaved_engine.a before this package is compiled.
import PackageDescription

let package = Package(
    name: "InterleavedStudy",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "CEngine", path: "Sources/CEngine"),
        .executableTarget(
            name: "InterleavedStudy",
            dependencies: ["CEngine"],
            path: "Sources/InterleavedStudy",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags(["-L", "build/lib"]),
                .linkedLibrary("interleaved_engine"),
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
                .linkedFramework("CoreFoundation"),
            ]
        ),
    ]
)
