import CoreGraphics
import Foundation

/// How tall an expanded message's body is drawn inside a conversation.
///
/// The pane stacks bodies, so each web view is sized to its content and never
/// scrolls on its own — one scroll view, the conversation's. That means the
/// height is *measured* from the laid-out document, and a measurement is a
/// number from outside the program: it can be zero because layout has not
/// happened yet, absurd because a sender said so, or `NaN` because WebKit
/// answered before there was a document to measure.
///
/// A frame is not the place to find that out. `NaN` in a layout constraint is
/// a crash rather than a bad height, so it is caught here, where it is three
/// lines and a test rather than a report about a message nobody can reopen.
public enum BodyHeight {
    /// Enough to say "there is a message here" when nothing measured.
    public static let minimum: CGFloat = 32

    /// The tallest body drawn in one piece. Past this the conversation's own
    /// scrolling takes over, which is what it is for.
    public static let maximum: CGFloat = 12_000

    /// `raw` as a height something can actually be drawn at.
    public static func clamped(_ raw: Double) -> CGFloat {
        guard raw.isFinite else {
            return raw > 0 ? maximum : minimum
        }
        return CGFloat(min(max(raw, Double(minimum)), Double(maximum)))
    }
}
