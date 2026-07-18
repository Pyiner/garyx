// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "GaryxMobileCore",
    platforms: [
        .iOS("26.0"),
        .macOS(.v14),
    ],
    products: [
        .library(
            name: "GaryxMobileCore",
            targets: ["GaryxMobileCore"]
        ),
        .executable(
            name: "GaryxComposerDurabilityCrashHarness",
            targets: ["GaryxComposerDurabilityCrashHarness"]
        ),
    ],
    targets: [
        .target(name: "GaryxMobileCore"),
        .executableTarget(
            name: "GaryxComposerDurabilityCrashHarness",
            dependencies: ["GaryxMobileCore"]
        ),
        .testTarget(
            name: "GaryxMobileCoreTests",
            dependencies: [
                "GaryxMobileCore",
                "GaryxComposerDurabilityCrashHarness",
            ],
            resources: [.copy("Fixtures")]
        ),
    ]
)
