// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "SSHApp",
    platforms: [
        .iOS(.v17),
        .macOS(.v14)
    ],
    products: [
        .library(
            name: "SSHApp",
            targets: ["SSHApp"]
        )
    ],
    dependencies: [
        .package(name: "SshCoreBridge", path: "../packages/SshCoreBridge"),
        .package(url: "https://github.com/migueldeicaza/SwiftTerm.git", from: "1.0.7")
    ],
    targets: [
        .target(
            name: "SSHApp",
            dependencies: [
                "SshCoreBridge",
                .product(name: "SwiftTerm", package: "SwiftTerm")
            ],
            path: "Sources/SSHApp"
        )
    ]
)
