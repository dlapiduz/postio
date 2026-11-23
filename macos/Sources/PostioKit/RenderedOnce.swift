/// `H` renders held-back parts for one message, once: nothing is written, no sender is allowed, and
/// closing the conversation forgets it. Kept apart from the standing grant (`allowRemoteImages`) so
/// "show this one" never becomes "trust this sender". A type, not `@State`, because `H` is a
/// command; the "Show images" button presses the same thing.
public struct RenderedOnce: Equatable, Sendable {
    private var showing: Set<Int64> = []

    public init() {}

    public func isOn(_ message: Int64) -> Bool { showing.contains(message) }

    /// One-way: the pictures have been fetched and the sender already knows, so un-rendering would
    /// be a promise Postio cannot keep.
    public mutating func render(_ message: Int64) {
        showing.insert(message)
    }

    public mutating func clear() { showing = [] }
}
