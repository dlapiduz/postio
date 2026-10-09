import Foundation
import Testing

/// Colour comes from a role, never from a literal (spec 009 FR-017,
/// contracts/mac-surfaces.md).
///
/// A hex string or an RGB initialiser in a view is a copy of a design token
/// that is right on the day it is written. Roles are `PostioTokens` (generated
/// from the design system) and the system's semantic colours, which follow
/// light and dark and the accessibility settings for free.
///
/// Scans `macos/Sources` except the generated bindings and `Generated/`,
/// which is where the literals are supposed to live. Comment lines are
/// skipped: they quote the token values they explain.
struct SemanticColourTests {
    /// Files allowed to break the rule, with the reason. Empty today: nothing
    /// under `Sources/` offends. A file that must join it carries the comment
    /// "classic three-pane code, deleted in US1 (T034)" or a better reason.
    static let allowed: Set<String> = []

    static let sources: URL = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // PostioKitTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .appendingPathComponent("Sources")

    static let skipped = ["/PostioFFI/", "/postio_ffiFFI/", "/Generated/"]

    static func files() -> [URL] {
        guard let walk = FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil)
        else { return [] }
        return walk.compactMap { $0 as? URL }
            .filter { url in
                url.pathExtension == "swift" && !skipped.contains { url.path.contains($0) }
            }
            .sorted { $0.path < $1.path }
    }

    /// `path:line: text` for each line that builds a colour from a literal.
    static func violations() throws -> [String] {
        let hex = try Regex(##""#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?""##)
        let rgb = try Regex(
            #"\b(Color|NSColor|UIColor)\((red|calibratedRed|srgbRed|displayP3Red):"#
        )
        var found: [String] = []
        for file in files() where !allowed.contains(file.lastPathComponent) {
            let text = try String(contentsOf: file, encoding: .utf8)
            for (number, line) in text.split(separator: "\n", omittingEmptySubsequences: false)
                .enumerated()
            {
                if line.trimmingCharacters(in: .whitespaces).hasPrefix("//") { continue }
                if line.firstMatch(of: hex) != nil || line.firstMatch(of: rgb) != nil {
                    found.append("\(file.lastPathComponent):\(number + 1): \(line)")
                }
            }
        }
        return found
    }

    @Test func theSourcesAreFoundSoTheCheckCanFail() {
        #expect(Self.files().count > 20)
    }

    @Test func noColourIsBuiltFromALiteral() throws {
        let found = try Self.violations()
        #expect(found.isEmpty, "use a PostioTokens role or a semantic colour: \(found)")
    }

    @Test func theMatcherSeesWhatItIsFor() throws {
        // The scan finds nothing today, so prove the patterns can fail.
        let hex = try Regex(##""#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?""##)
        let rgb = try Regex(#"\b(Color|NSColor|UIColor)\((red|calibratedRed|srgbRed|displayP3Red):"#)
        #expect("let c = Color(hex: \"#5980a6\")".firstMatch(of: hex) != nil)
        #expect("NSColor(srgbRed: 0.1, green: 0, blue: 0, alpha: 1)".firstMatch(of: rgb) != nil)
        #expect("Color(red: 1, green: 0, blue: 0)".firstMatch(of: rgb) != nil)
        #expect("Color.primary".firstMatch(of: rgb) == nil)
    }
}
