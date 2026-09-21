import CoreGraphics
import Observation

/// The measured height of each message body the reader has drawn before.
///
/// A body's height comes back from a `scrollHeight` round trip through the
/// web view, so the first time a message opens it draws at
/// `BodyHeight.minimum` and *pops* to full size a beat later. That first
/// measurement is inherent — HTML has no height until it is laid out — but
/// paying it again on every revisit is not: the same document lays out to
/// the same height, so a remembered one is right until the pane's width
/// changes, and the measurement still runs and corrects it when it is not.
///
/// Session-lived and never persisted: a height is a fact about this window's
/// width and this build's stylesheet, and a stored one would outlive both.
@MainActor
@Observable
public final class BodyHeights {
    /// Measured heights by message id.
    private var measured: [Int64: CGFloat] = [:]

    /// Past this many entries the cache is dropped whole rather than
    /// evicted piecemeal: it refills at one round trip per message, and an
    /// LRU here would be bookkeeping in service of avoiding work the pane
    /// does anyway.
    static let capacity = 512

    public init() {}

    /// What `message` measured last time, or `nil` for a first opening.
    public func height(for message: Int64) -> CGFloat? {
        measured[message]
    }

    /// Remember what `message` laid out to.
    public func remember(_ height: CGFloat, for message: Int64) {
        if measured.count >= Self.capacity, measured[message] == nil {
            measured.removeAll(keepingCapacity: true)
        }
        measured[message] = height
    }
}
