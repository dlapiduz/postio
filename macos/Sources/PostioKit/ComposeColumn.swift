import CoreGraphics
import PostioFFI

/// The composer's body column at the window's own width: spec 009 M1's
/// rule, `min(960, w - 80)`, extended from the message window to the
/// composer. It is the same shared geometry the message window asks
/// (`focusMessageColumn`), not a second copy of the numbers.
public enum ComposeColumn {
    /// The widest the body may be in a window `window` wide; unbounded
    /// until the window has a width.
    public static func width(window: CGFloat) -> CGFloat {
        guard window > 0 else { return .infinity }
        let column = focusMessageColumn(windowWidth: Int32(window.rounded()), treatment: .appColours)
        return CGFloat(column.columnWidth)
    }
}
