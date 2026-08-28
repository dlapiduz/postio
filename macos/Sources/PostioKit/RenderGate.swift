import Foundation

/// Decides whether a render that has come back is still the one wanted.
///
/// Building a reader document is a SQLite read, a sanitise and a wrap. Doing
/// that on the cursor's own thread makes every `j` cost a disk read, so it
/// happens off the main actor — and the moment it does, results can arrive out
/// of order. A body for the message the cursor has already left would draw one
/// message's text under another's header, which is the shape of #70 and the
/// reason `postio-app/src/reading.rs` carries the same apparatus.
///
/// A counter rather than comparing message ids: moving away and straight back
/// gives the same id twice, and the older render is still stale.
public final class RenderGate {
    private var current: UInt64 = 0

    public init() {}

    /// Claim the next render, and get the token that identifies it.
    public func begin() -> UInt64 {
        current += 1
        return current
    }

    /// Whether `token` is still the render being waited for.
    public func isCurrent(_ token: UInt64) -> Bool {
        token == current
    }
}
