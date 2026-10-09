import CoreGraphics

/// How a body drawn on paper fits its column (SPEC section 3, M5).
///
/// An HTML layout wider than its column is scaled down to fit -- a 640
/// newsletter in the 576 column a 1024-wide main window gives is drawn at
/// 0.9 -- and never below the engine's floor (`paperFloor`, 0.85): past it
/// the sheet keeps the floor and scrolls sideways rather than shrink type
/// out of reading. A layout that fits is drawn at its own size, never
/// enlarged.
///
/// The one geometry the Mac computes itself, because only the web view can
/// measure what the sender's layout came to; the column and the floor are
/// the engine's.
public enum PaperFit {
    /// The zoom for a document `measured` points wide in a `column`.
    public static func zoom(column: CGFloat, measured: CGFloat, floor: Double) -> Double {
        guard measured > 0, column > 0 else { return 1 }
        return min(1, max(floor, Double(column / measured)))
    }
}
