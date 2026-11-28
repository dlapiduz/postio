/// Every message opens as its sender built it; reader view (`⇧⌘O`) is per message and per view and
/// nothing is remembered. A type rather than `@State` because both are commands, which cannot reach
/// view state.
public struct ReaderViewChoice: Equatable, Sendable {
    private var reduced: Set<Int64> = []

    public init() {}

    public func isReduced(_ message: Int64) -> Bool { reduced.contains(message) }

    public var messages: [Int64] { reduced.sorted() }

    public mutating func toggle(_ message: Int64) {
        if reduced.contains(message) {
            reduced.remove(message)
        } else {
            reduced.insert(message)
        }
    }

    /// Not a toggle: over a message already shown as sent it does nothing.
    public mutating func showOriginal(_ message: Int64) {
        reduced.remove(message)
    }

    public mutating func clear() { reduced = [] }
}
