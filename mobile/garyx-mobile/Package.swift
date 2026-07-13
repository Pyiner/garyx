// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "GaryxMobileCore",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(
            name: "GaryxMobileCore",
            targets: ["GaryxMobileCore"]
        ),
    ],
    targets: [
        .target(name: "GaryxMobileCore"),
        .testTarget(
            name: "GaryxMobileCoreTests",
            dependencies: ["GaryxMobileCore"],
            resources: [.copy("Fixtures")]
        ),
    ]
)
