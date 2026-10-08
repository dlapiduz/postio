import CoreGraphics
import Foundation
import PostioFFI

/// Running over one of Focus's demo stores, for screenshots and the
/// comparison with the design (specs/009-focus-macos T004, FR-061).
///
/// `POSTIO_DEMO=<seed>` opens the in-memory store `postio-demo` builds --
/// "small", "thirty-threads", ... -- instead of the store on disk, so no
/// Keychain is read and nobody's mail is shown. Only a build with the FFI's
/// `demo` feature can open one; any other says so on launch.
/// `POSTIO_WINDOW_SIZE=WIDTHxHEIGHT` sizes the main window, which otherwise
/// opens at the design's 1440 × 900 and does not restore a saved frame.
/// `POSTIO_DEMO_KEYS="! x j x"` presses those keys on the list once its
/// first page has landed, so a screen that needs a state -- `!` on, three
/// rows marked -- can be photographed; only in a demo.
public enum DemoMode {
    /// The demo store asked for, if any.
    public static var seed: String? { seed(in: ProcessInfo.processInfo.environment) }

    /// The main window's size in demo mode.
    public static var windowSize: CGSize {
        windowSize(in: ProcessInfo.processInfo.environment)
    }

    /// "light" or "dark" from `POSTIO_APPEARANCE`, to photograph either
    /// whatever the system is set to; nil to follow the system.
    public static var appearance: String? {
        appearance(in: ProcessInfo.processInfo.environment)
    }

    /// The keys to press on the list once it has landed, in a demo only.
    public static var keys: [KeyEvent.Reduced] { keys(in: ProcessInfo.processInfo.environment) }

    static func keys(in environment: [String: String]) -> [KeyEvent.Reduced] {
        guard seed(in: environment) != nil, let said = environment["POSTIO_DEMO_KEYS"] else {
            return []
        }
        // Each word one key, as typed on a US keyboard: a capital or a
        // shifted symbol has Shift held, which is what the resolver sees
        // from a real press.
        return said.split(separator: " ").map { word in
            var key = Substring(word)
            // `⌘` and `⌥` before a key hold them: `⌘k` opens the command
            // bar, `⌘⌫` takes its chips back to words, `⌥1` runs a saved
            // search.
            var command = false
            var option = false
            while key.count > 1, let held = key.first, held == "⌘" || held == "⌥" {
                if held == "⌘" { command = true } else { option = true }
                key = key.dropFirst()
            }
            let typed = String(key)
            // Return, Escape, Tab and the arrows by their glyphs: named
            // keys, as `KeyEvent.reduce` reports them from a real press.
            if let name = namedKeys[typed] {
                return KeyEvent.Reduced(
                    character: nil, name: name,
                    modifiers: ModifiersFfi(control: false, option: option, shift: false, command: command))
            }
            // `␣` is the space a list of words cannot hold.
            if typed == "␣" {
                return KeyEvent.Reduced(
                    character: " ", name: nil,
                    modifiers: ModifiersFfi(control: false, option: false, shift: false, command: false))
            }
            // A word of several characters is typed whole into a field the
            // bar or the popover holds; on the list it resolves to nothing.
            let shifted = typed.count == 1
                && (typed != typed.lowercased() || "!@#$%^&*()_+{}|:\"<>?~".contains(typed))
            return KeyEvent.Reduced(
                character: typed, name: nil,
                modifiers: ModifiersFfi(control: false, option: option, shift: shifted, command: command))
        }
    }

    /// The glyphs a replay may name a key by.
    static let namedKeys = [
        "⏎": "return", "↩": "return", "⎋": "escape", "⇥": "tab", "⌫": "backspace",
        "↓": "down", "↑": "up",
    ]

    static func appearance(in environment: [String: String]) -> String? {
        environment["POSTIO_APPEARANCE"].flatMap { ["light", "dark"].contains($0) ? $0 : nil }
    }

    static func seed(in environment: [String: String]) -> String? {
        environment["POSTIO_DEMO"].flatMap { $0.isEmpty ? nil : $0 }
    }

    static func windowSize(in environment: [String: String]) -> CGSize {
        let parts = (environment["POSTIO_WINDOW_SIZE"] ?? "")
            .split(separator: "x")
            .compactMap { Double($0) }
        guard parts.count == 2, parts.allSatisfy({ $0 > 0 }) else {
            return CGSize(width: 1440, height: 900)
        }
        return CGSize(width: parts[0], height: parts[1])
    }
}
