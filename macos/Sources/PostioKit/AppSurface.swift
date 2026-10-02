import AppKit

/// What the window's own surfaces are painted with (#1588).
///
/// **Postio's neutral ramp is pure neutral** — red and green are equal at
/// every step — and macOS's dark chrome is a blue-grey about seven units
/// more blue than red. Until this existed the panes were AppKit's and the
/// reader's body was Postio's, so a pure-neutral panel sat inside a blue-grey
/// surround and simultaneous contrast made it read *maroon*. Nothing was
/// drawing red; there is not one token in the design system where red
/// exceeds blue. The reader was simply the only large surface still on
/// Postio's ramp.
///
/// The same defect `MessageRowView` records one file over: a surface with no
/// Postio colour shows the platform's default, and the platform's default is
/// on a different ramp.
///
/// # Why dark is derived rather than designed
///
/// The design system has no dark mode — `prefers-color-scheme` appears zero
/// times in `Design/_ds/industry-*/styles.css`. It defines one light palette
/// whose darkest neutral is `--color-neutral-900`, which is also where the
/// reader's dark ground came from.
///
/// So dark is derived here, and only its *level* is: the hue comes from the
/// token, and scaling a neutral keeps it neutral. Light is the designed
/// relationship — `--color-bg` sits three steps below `--color-neutral-100`,
/// so the reader's paper reads a shade lighter than the window behind it —
/// and dark mirrors it. #1588 is where a designed dark palette replaces this.
public enum AppSurface {
    /// How far below the reader's paper the window sits, in 8-bit steps.
    ///
    /// The light palette's own gap: `--color-bg` (#f2f2f3, 242) is three
    /// steps under `--color-neutral-100` (#f5f5f8, 245). Mirrored rather
    /// than invented, which is the most that can honestly be derived without
    /// a dark plate to read.
    ///
    /// **Steps, not a ratio.** The same *proportion* — 242/245 is about 1.2%
    /// — comes to half a unit at the dark end, which is below what eight
    /// bits can express: the window and the paper round to the same value
    /// and the card flattens. A gap is a gap at either end of the ramp.
    static let stepsBelowPaper: CGFloat = 3.0 / 255.0

    /// The window and its panes.
    ///
    /// Light is `--color-bg` as drawn. Dark is the reader's own ground taken
    /// one step down, so the paper still reads as paper.
    public static let background = NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? lowered(PostioTokens.colorNeutral900, by: stepsBelowPaper)
            : PostioTokens.colorBg
    }

    /// The sidebar, one step further down again.
    ///
    /// The canvas gives the sidebar its own weight — it is the frame around
    /// the mail rather than a pane of it — and the window's own hierarchy is
    /// worth keeping while the ramp is put right. Same derivation, same
    /// neutrality.
    public static let sidebar = NSColor(name: nil) { appearance in
        appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
            ? lowered(PostioTokens.colorNeutral900, by: stepsBelowPaper * 2)
            : lowered(PostioTokens.colorBg, by: stepsBelowPaper)
    }

    /// `color` moved toward black by `amount`, which leaves its hue alone.
    ///
    /// Subtracting the same amount from every channel cannot introduce a
    /// cast: a colour whose red and green are equal still has them equal
    /// afterwards, and one with a blue lift keeps exactly that lift. That is
    /// the whole reason the *level* may be derived here while the hue may
    /// not.
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
