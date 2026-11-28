import AppKit
import PostioKit

public enum Motion {
    /// Zero rather than "very fast": a 50 ms slide is still movement for someone who asked for
    /// Reduce Motion.
    public static func duration(reduceMotion: Bool) -> Double {
        reduceMotion ? 0 : 0.1
    }

    /// Read at the moment of use: the preference can change while Postio runs.
    @MainActor
    public static var current: Double {
        duration(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
    }
}
