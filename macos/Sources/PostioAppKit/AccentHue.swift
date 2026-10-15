import AppKit
import PostioKit

/// Tells `LabelColour` the hue of the user's accent, so a label with no
/// colour of its own is never drawn in it (FR-091): read once at launch and
/// again whenever the system colours change.
@MainActor
public enum AccentHue {
    private static var observer: NSObjectProtocol?

    /// Read the accent now, and follow it from here on.
    public static func follow() {
        read()
        guard observer == nil else { return }
        observer = NotificationCenter.default.addObserver(
            forName: NSColor.systemColorsDidChangeNotification, object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated { read() }
        }
    }

    private static func read() {
        guard let accent = NSColor.controlAccentColor.usingColorSpace(.sRGB) else { return }
        LabelColour.accentHue = Double(accent.hueComponent) * 360
    }
}
