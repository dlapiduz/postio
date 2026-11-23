import AppKit
import PostioKit

/// Postio's neutral ramp is pure neutral while macOS's dark chrome is blue-grey, so a pure-neutral
/// panel in a blue-grey surround reads maroon by contrast (#1588).
/// Dark is derived, and only its level: the design system has no dark mode, the hue comes from the
/// token, and scaling a neutral keeps it neutral. #1588 replaces this with a designed dark palette.
public enum AppSurface {
    /// Steps, not a ratio: the same proportion is half a unit at the dark end, below what eight
    /// bits can express, and the card would flatten.
    static let stepsBelowPaper: CGFloat = 3.0 / 255.0

    public static let background = NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? lowered(PostioTokens.colorNeutral900, by: stepsBelowPaper)
            : PostioTokens.colorBg
    }

    public static let sidebar = NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? lowered(PostioTokens.colorNeutral900, by: stepsBelowPaper * 2)
            : lowered(PostioTokens.colorBg, by: stepsBelowPaper)
    }

    /// Subtracting the same amount from every channel adds no cast, so the level may be derived
    /// while the hue may not.
    static func lowered(_ color: NSColor, by amount: CGFloat) -> NSColor {
        guard let c = color.usingColorSpace(.sRGB) else { return color }
        return NSColor(
            srgbRed: max(0, c.redComponent - amount),
            green: max(0, c.greenComponent - amount),
            blue: max(0, c.blueComponent - amount),
            alpha: c.alphaComponent
        )
    }
}
