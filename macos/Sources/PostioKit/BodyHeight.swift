import CoreGraphics
import Foundation

/// A measurement is a number from outside the program: zero before layout, absurd if a sender says
/// so, or `NaN` before there is a
/// document. `NaN` in a layout constraint crashes, so it is caught here.
public enum BodyHeight {
    public static let minimum: CGFloat = 32

    public static let maximum: CGFloat = 12_000

    public static func clamped(_ raw: Double) -> CGFloat {
        guard raw.isFinite else {
            return raw > 0 ? maximum : minimum
        }
        return CGFloat(min(max(raw, Double(minimum)), Double(maximum)))
    }
}
