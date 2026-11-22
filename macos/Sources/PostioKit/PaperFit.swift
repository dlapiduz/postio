import CoreGraphics

/// An HTML layout wider than its column scales down to fit, never below the engine's `paperFloor`
/// (it scrolls sideways instead), and is never enlarged. Computed here because only the web view
/// can measure the sender's layout.
public enum PaperFit {
    public static func zoom(column: CGFloat, measured: CGFloat, floor: Double) -> Double {
        guard measured > 0, column > 0 else { return 1 }
        return min(1, max(floor, Double(column / measured)))
    }
}
