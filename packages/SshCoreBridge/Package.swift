// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "SshCoreBridge",
    platforms: [
        .iOS(.v17),
        .macOS(.v14)
    ],
    products: [
        .library(
            name: "SshCoreBridge",
            targets: ["SshCoreBridge"]
        )
    ],
    targets: [
        .binaryTarget(
            name: "ssh_coreFFI",
            path: "Frameworks/ssh_coreFFI.xcframework"
        ),
        .target(
            name: "SshCoreBridge",
            dependencies: ["ssh_coreFFI"],
            path: "Sources/SshCoreBridge"
        ),
        .testTarget(
            name: "SshCoreBridgeTests",
            dependencies: ["SshCoreBridge"],
            path: "Tests/SshCoreBridgeTests"
        )
    ]
)
