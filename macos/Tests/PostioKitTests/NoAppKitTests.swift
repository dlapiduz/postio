import Foundation
import Testing

/// PostioKit must not require AppKit (#1264, spec 009 R10).
///
/// The package is meant to be able to reach iOS, and one `import AppKit` in
/// the models is enough to stop that. Anything that needs AppKit lives in
/// `PostioAppKit`, which depends on this target and never the other way
/// round. This reads the source because the compiler cannot say it: on macOS
/// an `import AppKit` always builds.
///
/// `Generated/` is skipped. `Tokens.swift` is emitted by
/// `postio-tokens` (crates/postio-ui), is gitignored, and still emits
/// `NSColor`; making the generator platform-neutral is a change to that
/// crate, not to this package.
struct NoAppKitTests {
    /// `macos/Sources/PostioKit`, found from this file rather than the
    /// working directory, which `swift test` does not fix.
    static let kitSources: URL = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // PostioKitTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .appendingPathComponent("Sources/PostioKit")

    static func swiftFiles(under root: URL) -> [URL] {
        guard let walk = FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil)
        else { return [] }
        return walk.compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" && !$0.path.contains("/Generated/") }
            .sorted { $0.path < $1.path }
    }

    @Test func theSourcesAreFoundSoTheCheckCanFail() {
        #expect(Self.swiftFiles(under: Self.kitSources).count > 20)
    }

    @Test func noPostioKitFileImportsAppKit() throws {
        let banned = try Regex(#"(?m)^\s*(@\w+\s+)*import\s+(AppKit|Cocoa)\b"#)
        var offenders: [String] = []
        for file in Self.swiftFiles(under: Self.kitSources) {
            let text = try String(contentsOf: file, encoding: .utf8)
            if text.firstMatch(of: banned) != nil {
                offenders.append(file.lastPathComponent)
            }
        }
        #expect(
            offenders.isEmpty,
            "PostioKit imports AppKit in \(offenders); move the file to PostioAppKit (#1264)"
        )
    }
}
