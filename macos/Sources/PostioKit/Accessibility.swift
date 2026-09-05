import AppKit
import PostioFFI

/// What Postio says to a screen reader, and how much it moves.
///
/// `docs/PRODUCT.md` §20 makes accessibility first-class, and the GTK side
/// paid for that with real work — the reader's web view declares an article
/// role, and the custom-drawn rows expose properties that exist only because
/// somebody added them. A second frontend that skipped this would be shipping
/// a mail client a blind person cannot use, on the platform whose screen
/// reader is built in.
///
/// The decisions are here, as pure functions, for the reason
/// `docs/engineering-notes.md` records about the GTK side: **GTK records no
/// accessible properties without a live backend**, which cost a whole
/// debugging session to learn. AppKit has the same shape of problem —
/// `accessibilityLabel` reads back as whatever was last set, whether or not
/// anything would ever speak it. So what is asserted is the *sentence*, which
/// is the part that can be wrong.
public enum Announcements {
    /// One row of the list, as one useful utterance.
    ///
    /// **One sentence, not four labels.** A row that exposes sender, subject,
    /// preview and unread state as separate elements makes VoiceOver read four
    /// things and makes arrowing through a mailbox four times as slow; a row
    /// that exposes the whole cell's text reads a wall including the preview.
    /// This is the middle: who it is from, what it is about, and the states
    /// that change what you would do about it.
    ///
    /// The preview is deliberately left out. It is a fragment of the body,
    /// often mid-sentence, and it is what the reading pane is for.
    public static func row(_ presentation: RowPresentation) -> String {
        if presentation.isPlaceholder {
            // Not silence: an unlabelled row reads as "row" and sounds like a
            // bug. "Loading" is what is actually happening.
            return "Loading"
        }
        var parts = [presentation.sender, presentation.subject]
        // States first among the trailing detail, because they are what
        // decides whether you stop here.
        if presentation.unread { parts.append("unread") }
        if presentation.flagged { parts.append("flagged") }
        if presentation.selected { parts.append("selected") }
        if let badge = presentation.threadBadge {
            parts.append("\(badge) messages")
        }
        return parts.joined(separator: ", ")
    }
}

/// The three panes, in the order the keyboard walks them.
///
/// The order is the *visual* one — sidebar, list, reader — because a focus
/// order that disagrees with the layout is the classic way a keyboard-first
/// application becomes unusable without a mouse. It is here rather than in a
/// view so it can be asserted; a cycle that skipped a pane or looped early is
/// invisible in a screenshot.
public enum Pane: CaseIterable, Sendable {
    case sidebar
    case list
    case reader

    /// The pane after this one, wrapping.
    ///
    /// Wrapping rather than stopping: `cycle_pane` is a cycle, and a user who
    /// has tabbed to the reader expects one more press to come back rather
    /// than to do nothing.
    public func next(_ forward: Bool = true) -> Pane {
        let all = Pane.allCases
        let at = all.firstIndex(of: self) ?? 0
        let moved = forward ? at + 1 : at - 1 + all.count
        return all[moved % all.count]
    }

    /// The surface this pane resolves keys as.
    ///
    /// The keyboard's context follows focus, or a key pressed in the sidebar
    /// would resolve against the list — which is how `j` ends up moving the
    /// wrong thing.
    public var context: UiContext {
        switch self {
        case .sidebar: return .sidebar
        case .list: return .list
        case .reader: return .reader
        }
    }

    /// What a screen reader calls it.
    public var label: String {
        switch self {
        case .sidebar: return "Folders"
        case .list: return "Messages"
        case .reader: return "Message"
        }
    }
}

/// The commands this frontend presents a surface for, rather than sending on.
///
/// Almost everything goes to `invoke`, where the boundary decides whether it
/// is its own or the engine's. These are the exceptions: each one *is* a
/// window, and a session cannot present one. `postio-gtk`'s `run_action` makes
/// the same call for the same reason.
///
/// They are named here rather than written as literals at the `switch`,
/// because a literal that no longer matches the registry is a key that
/// silently does nothing — `/` not opening search, with no error anywhere to
/// say why — and `everyInterceptedCommandIsInTheRegistry` is what notices.
/// Keeping the list short is what stops it becoming the hand-maintained
/// command table #657 exists to prevent.
public enum Intercepted {
    public static let palette = "command_palette"
    public static let cheatSheet = "cheat_sheet"
    public static let search = "search"
    public static let back = "back"
    public static let cyclePane = "cycle_pane"
    public static let cyclePaneBack = "cycle_pane_back"
    public static let focusSidebar = "focus_sidebar"

    /// Every id above, for the test that checks they still exist.
    public static let all = [
        palette, cheatSheet, search, back, cyclePane, cyclePaneBack, focusSidebar,
    ]
}

/// How long a transition may take.
///
/// `PRODUCT.md` §18: transitions are ≤100 ms or absent, and the preference is
/// honoured. On macOS that preference is
/// `NSWorkspace.shared.accessibilityDisplayShouldReduceMotion`.
public enum Motion {
    /// The budget, in seconds. Zero means "do it, do not animate it".
    ///
    /// Zero rather than "very fast": Reduce Motion is asked for by people for
    /// whom movement is a symptom, and a 50 ms slide is still movement. The
    /// state change still happens — what goes is the travel.
    public static func duration(reduceMotion: Bool) -> Double {
        reduceMotion ? 0 : 0.1
    }

    /// The budget for this machine, right now.
    ///
    /// Read at the moment of use rather than cached: the preference can be
    /// changed while Postio is running, and a cached copy would keep animating
    /// for somebody who had just asked it to stop.
    @MainActor
    public static var current: Double {
        duration(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
    }
}
