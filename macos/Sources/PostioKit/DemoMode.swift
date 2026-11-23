import CoreGraphics
import Foundation
import PostioFFI

/// `POSTIO_DEMO=<seed>` opens postio-demo's in-memory store instead of the one on disk, so no
/// Keychain is read and nobody's mail is shown; only a build with the FFI `demo` feature can.
/// `POSTIO_WINDOW_SIZE`, `POSTIO_DEMO_KEYS`, `POSTIO_DEMO_STATE` and `POSTIO_DEMO_SNAPSHOT` also
/// apply, in a demo only.
public enum DemoMode {
    public static var seed: String? { seed(in: ProcessInfo.processInfo.environment) }

    public static var windowSize: CGSize {
        windowSize(in: ProcessInfo.processInfo.environment)
    }

    /// Demo only: a picture of the dry run has no pointer to press the button with. The demo's
    /// address book is a fake.
    public static var previewsSync: Bool {
        seed != nil && ProcessInfo.processInfo.environment["POSTIO_DEMO_SYNC_PREVIEW"] == "1"
    }

    public static var appearance: String? {
        appearance(in: ProcessInfo.processInfo.environment)
    }

    public static var state: String? { state(in: ProcessInfo.processInfo.environment) }

    public static var keys: [KeyEvent.Reduced] { keys(in: ProcessInfo.processInfo.environment) }

    static func keys(in environment: [String: String]) -> [KeyEvent.Reduced] {
        guard seed(in: environment) != nil, let said = environment["POSTIO_DEMO_KEYS"] else {
            return []
        }
        return said.split(separator: " ").map { word in
            var key = Substring(word)
            var command = false
            var option = false
            while key.count > 1, let held = key.first, held == "⌘" || held == "⌥" {
                if held == "⌘" { command = true } else { option = true }
                key = key.dropFirst()
            }
            let typed = String(key)
            if let name = namedKeys[typed] {
                return KeyEvent.Reduced(
                    character: nil, name: name,
                    modifiers: ModifiersFfi(control: false, option: option, shift: false, command: command))
            }
            if typed == "␣" {
                return KeyEvent.Reduced(
                    character: " ", name: nil,
                    modifiers: ModifiersFfi(control: false, option: false, shift: false, command: false))
            }
            let shifted = typed.count == 1
                && (typed != typed.lowercased() || "!@#$%^&*()_+{}|:\"<>?~".contains(typed))
            return KeyEvent.Reduced(
                character: typed, name: nil,
                modifiers: ModifiersFfi(control: false, option: option, shift: shifted, command: command))
        }
    }

    static let namedKeys = [
        "⏎": "return", "↩": "return", "⎋": "escape", "⇥": "tab", "⌫": "backspace",
        "↓": "down", "↑": "up", "→": "right", "←": "left",
    ]

    /// Drawn by the app itself, for a terminal `screencapture` cannot photograph (no Screen
    /// Recording grant); materials and shadows may differ.
    public static var snapshot: String? { snapshot(in: ProcessInfo.processInfo.environment) }

    static func snapshot(in environment: [String: String]) -> String? {
        guard seed(in: environment) != nil else { return nil }
        return environment["POSTIO_DEMO_SNAPSHOT"].flatMap { $0.isEmpty ? nil : $0 }
    }

    static func state(in environment: [String: String]) -> String? {
        guard seed(in: environment) != nil else { return nil }
        return environment["POSTIO_DEMO_STATE"].flatMap { $0.isEmpty ? nil : $0 }
    }

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
