/// Which messages have had their held-back parts rendered for this view.
///
/// The reader blocks remote images and the trackers hiding among them, and
/// `H` — *Render part once* — is the escape hatch for the one message in
/// front of you. **Once** is the whole of it: nothing is written, no sender
/// is allowed, and closing the conversation forgets it. The standing grant is
/// a different gesture with a different surface (`allowRemoteImages`), and
/// keeping them apart is what stops "just show me this one" from quietly
/// becoming "trust everything this sender ever sends".
///
/// A type rather than an `@State` in the view, for the reason `OriginalView`
/// records: `H` is a command, a command cannot reach view state, and a flag
/// that lives only inside a `View` is a flag no test can look at. The "Show
/// images" button in the blocked-images notice presses the same thing, so the
/// key and the button cannot drift.
public struct RenderedOnce: Equatable, Sendable {
    private var showing: Set<Int64> = []

    public init() {}

    /// Whether `message` is drawing what was held back.
    public func isOn(_ message: Int64) -> Bool { showing.contains(message) }

    /// Render `message`'s held-back parts.
    ///
    /// One-way, unlike [`OriginalView.toggle`]. Un-rendering would be a
    /// promise Postio cannot keep: the pictures have been fetched, the sender
    /// already knows the message was opened, and hiding them again would say
    /// otherwise.
    public mutating func render(_ message: Int64) {
        showing.insert(message)
    }

    /// Forget everything — the pane is showing something else now.
    public mutating func clear() { showing = [] }
}
