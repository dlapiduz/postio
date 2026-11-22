import Foundation

/// Building a reader document runs off the main actor, so results can arrive out of order and a
/// body for a message the cursor left would draw under another's header (#70). A counter, not
/// message ids: moving away and back gives the same id twice and the older render is still stale.
public final class RenderGate {
    private var current: UInt64 = 0

    public init() {}

    public func begin() -> UInt64 {
        current += 1
        return current
    }

    public func isCurrent(_ token: UInt64) -> Bool {
        token == current
    }
}
