// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "MewMobileCore",
    platforms: [
        .iOS(.v17),
    ],
    products: [
        .library(
            name: "MewMobileCore",
            targets: ["MewMobileCore"]
        ),
    ],
    targets: [
        // The static library + FFI headers, wrapped as an xcframework binary target.
        // The modulemap inside defines the `mew_mobile_coreFFI` module.
        .binaryTarget(
            name: "mew_mobile_coreFFI",
            path: "XCFramework/mew_mobile_core.xcframework"
        ),
        // The generated Swift bindings, which import mew_mobile_coreFFI.
        .target(
            name: "MewMobileCore",
            dependencies: ["mew_mobile_coreFFI"],
            path: "Sources/MewMobileCore",
            exclude: ["mew_mobile_coreFFI.h", "mew_mobile_coreFFI.modulemap"]
        ),
    ]
)
