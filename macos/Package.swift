// swift-tools-version: 6.0
import PackageDescription

// SwiftPM rather than a checked-in `.xcodeproj` (ADR 0019 Q8). A `.pbxproj` is
// a merge-hostile XML blob, and this repository is script-driven and largely
// agent-written; the packaging is still Ghostty's — a Rust staticlib behind a
// module map, linked into a Swift app.
//
// `.unsafeFlags` is deliberately *not* used here. It would pin an absolute
// path into the manifest and would bar this package from ever being a
// dependency; the library search path comes from `scripts/macos-build.sh` via
// `-Xlinker` instead, which keeps the manifest portable and the path in one
// place.
let package = Package(
    name: "Postio",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Postio", targets: ["Postio"]),
        .library(name: "PostioKit", targets: ["PostioKit"]),
    ],
    targets: [
        // The C side of the boundary: the header and module map `uniffi`
        // generates. Written by `scripts/ffi-bindgen.sh` and gitignored — a
        // build product, like the staticlib it describes.
        .systemLibrary(name: "postio_ffiFFI", path: "Sources/postio_ffiFFI"),

        // The Swift side of the boundary, also generated. Kept as its own
        // target so nothing hand-written shares a directory with something a
        // build step overwrites.
        .target(
            name: "PostioFFI",
            dependencies: ["postio_ffiFFI"],
            path: "Sources/PostioFFI"
        ),

        // Everything hand-written that is not a view.
        .target(name: "PostioKit", dependencies: ["PostioFFI"]),

        .executableTarget(name: "Postio", dependencies: ["PostioKit"]),

        .testTarget(name: "PostioKitTests", dependencies: ["PostioKit"]),
    ]
)
