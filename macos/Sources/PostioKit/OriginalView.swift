/// Which messages are being shown as their sender wrote them.
///
/// *View original* is the one gesture that leaves reader view, and it is
/// **per message and per view**: nothing is remembered, so the next message
/// opens reduced again. Bulk mail is what reader view is for — nested tables
/// are what a campaign template does and a person writing mail does not — and
/// a standing "show me originals" would quietly turn it off for everything.
///
/// A type rather than an `@State` in the view, because `⌘O` is a command and
/// a command cannot reach view state. That is the whole of why the key did
/// nothing while the `⋯` menu item beside it worked.
public struct OriginalView: Equatable, Sendable {
    private var showing: Set<Int64> = []

    public init() {}

    /// Whether `message` is drawn as it arrived.
    public func isOn(_ message: Int64) -> Bool { showing.contains(message) }

    /// Turn it on, or off again.
    public mutating func toggle(_ message: Int64) {
        if showing.contains(message) {
            showing.remove(message)
        } else {
            showing.insert(message)
        }
    }

    /// Forget everything — the pane is showing something else now.
    public mutating func clear() { showing = [] }
}
