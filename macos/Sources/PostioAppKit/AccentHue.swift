import AppKit
import PostioKit

@MainActor
public enum AccentHue {
    private static var observer: NSObjectProtocol?

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
