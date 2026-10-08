import CoreGraphics
import Foundation

/// Running over one of Focus's demo stores, for screenshots and the
/// comparison with the design (specs/009-focus-macos T004, FR-061).
///
/// `POSTIO_DEMO=<seed>` opens the in-memory store `postio-demo` builds --
/// "small", "thirty-threads", ... -- instead of the store on disk, so no
/// Keychain is read and nobody's mail is shown. Only a build with the FFI's
/// `demo` feature can open one; any other says so on launch.
/// `POSTIO_WINDOW_SIZE=WIDTHxHEIGHT` sizes the main window, which otherwise
/// opens at the design's 1440 × 900 and does not restore a saved frame.
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
