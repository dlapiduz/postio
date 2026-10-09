import Foundation

/// "Open that window", said in a way SwiftUI can hear twice.
///
/// A window is opened by calling `openWindow(id:)`, which only a view can do,
/// so a command that means "open the settings window" has to travel from
/// wherever it was run to a view that is on screen. The obvious spelling —
/// a `Bool` the view watches — cannot say *again*: SwiftUI's `onChange`
/// compares values, so a second `⌘,` while the flag is already `true` looks
/// like nothing happened, and the window that was closed stays closed.
///
/// So a request is a count. Two requests are two openings, whatever happened
/// in between. `Engine.requested` learned the same thing about notification
/// clicks; this is that shape with a name.
public struct WindowRequest: Equatable, Sendable {
    /// Which window, as `openWindow(id:)` names it.
    public let id: String
    /// How many times it has been asked for.
    public private(set) var count: Int

    public init(id: String) {
        self.id = id
        count = 0
    }

    /// Ask for the window.
    public mutating func raise() {
        count += 1
    }

    /// Whether anybody has asked yet — a view watching the count must not
    /// open a window on the first render because the count started at zero.
    public var wasRaised: Bool { count > 0 }
}

/// The windows this application can be asked to open by id.
public enum WindowId {
    /// Settings. Its own window with traffic lights, not a sheet or a pane:
    /// `⌘,` has opened one on this platform since Mac OS X 10.0, and ADR 0031
    /// keeps the model shared with GTK while the frame differs.
    public static let settings = "settings"
}
