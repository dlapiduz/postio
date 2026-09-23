import AppKit

/// The selected row, drawn the way the canvas draws it.
///
/// `NSTableView`'s own highlight is a full-width system-blue fill, and Postio
/// draws neither: the design system says *"airy rows, a 3px steel edge when
/// selected"* — a 12% accent tint with an accent edge down the leading side.
/// GTK has drawn that since it had rows; macOS drew AppKit's blue, because
/// `generate_swift` emitted no selection colour for it to use. The tokens are
/// derived rather than declared in `:root`, so the emitter's loop never saw
/// them.
///
/// Both colours now come from the same `:root` the CSS does, through
/// `PostioTokens`, so retuning the canvas still moves both frontends at once
/// — which is the whole point of the tokens being generated.
final class MessageRowView: NSTableRowView {
    /// The width of the leading edge, per the design system's own wording.
    static let edge: CGFloat = 3

    override func drawSelection(in dirtyRect: NSRect) {
        // `.none` on the table would lose the keyboard's row highlight too;
        // overriding the drawing keeps every other table behaviour — the
        // cursor, scrolling to it, accessibility — and changes only the paint.
        guard selectionHighlightStyle != .none else { return }
        PostioTokens.colorSelectedBg.setFill()
        bounds.fill()
        PostioTokens.colorAccent.setFill()
        NSRect(x: bounds.minX, y: bounds.minY, width: Self.edge, height: bounds.height).fill()
    }

    /// No separator: the rows are airy and the design draws none between them.
    override func drawSeparator(in dirtyRect: NSRect) {}
}
