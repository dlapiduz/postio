import Foundation

/// A count, not a `Bool`: SwiftUI's `onChange` compares values, so a second `⌘,` with the flag
/// already `true` would look like nothing happened. Two requests are two openings.
public struct WindowRequest: Equatable, Sendable {
    public let id: String
    public private(set) var count: Int

    public init(id: String) {
        self.id = id
        count = 0
    }

    public mutating func raise() {
        count += 1
    }

    /// A view watching the count must not open a window on first render because it started at zero.
    public var wasRaised: Bool { count > 0 }
}

public enum WindowId {
    /// Its own window, not a sheet or a pane; ADR 0031 keeps the model shared with GTK while the
    /// frame differs.
    public static let settings = "settings"
}
