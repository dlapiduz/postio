/// Which messages the reader asked to see in reader view.
///
/// Every message opens as its sender built it (spec 006 FR-031) -- the rule
/// GTK and the shared thread page follow, and since #1705 the Mac's
/// single-message pane too, which had opened bulk mail reduced. Reader view
/// is `⇧⌘O`, **per message and per view**: nothing is remembered, so the next
/// conversation opens as sent again. *View original* (`⌘O`) is the way back.
///
/// A type rather than an `@State` in the view, because both are commands and
/// a command cannot reach view state -- the reason `⌘O` once did nothing
/// while the `⋯` menu item beside it worked.
public struct ReaderViewChoice: Equatable, Sendable {
    private var reduced: Set<Int64> = []

    public init() {}

    /// Whether `message` is drawn reduced.
    public func isReduced(_ message: Int64) -> Bool { reduced.contains(message) }

    /// The messages drawn reduced, in a stable order -- what the boundary is
    /// asked to compose the thread page with.
    public var messages: [Int64] { reduced.sorted() }

    /// `toggle_reader_view`: reduce `message`, or draw it as sent again.
    public mutating func toggle(_ message: Int64) {
        if reduced.contains(message) {
            reduced.remove(message)
        } else {
            reduced.insert(message)
        }
    }

    /// `view_original`: draw `message` as sent. Not a toggle -- over a message
    /// already shown as sent it does nothing.
    public mutating func showOriginal(_ message: Int64) {
        reduced.remove(message)
    }

    /// Forget everything -- the pane is showing something else now.
    public mutating func clear() { reduced = [] }
}
