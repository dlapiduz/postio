import AppKit
import PostioKit

/// How long a transition may take.
///
/// `PRODUCT.md` §18: transitions are ≤100 ms or absent, and the preference is
/// honoured. On macOS that preference is
/// `NSWorkspace.shared.accessibilityDisplayShouldReduceMotion`.
public enum Motion {
    /// The budget, in seconds. Zero means "do it, do not animate it".
    ///
    /// Zero rather than "very fast": Reduce Motion is asked for by people for
    /// whom movement is a symptom, and a 50 ms slide is still movement. The
    /// state change still happens — what goes is the travel.
    public static func duration(reduceMotion: Bool) -> Double {
        reduceMotion ? 0 : 0.1
    }

    /// The budget for this machine, right now.
    ///
    /// Read at the moment of use rather than cached: the preference can be
    /// changed while Postio is running, and a cached copy would keep animating
    /// for somebody who had just asked it to stop.
    @MainActor
    public static var current: Double {
        duration(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
    }
}
