import Foundation
import Testing

struct NoAppKitTests {
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
