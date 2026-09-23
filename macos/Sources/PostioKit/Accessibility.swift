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
    /// The engine's table (`postio_ui::focus::next_pane`, through the
    /// boundary's `nextPane`), the same one the GTK window walks, so Tab
    /// means one thing on both. Wrapping rather than stopping: a user who
    /// has tabbed to the reader expects one more press to come back rather
    /// than to do nothing.
    public func next(_ forward: Bool = true) -> Pane {
        // A pane's context is always a pane, so the boundary always answers;
        // `self` is only the type checker's fallback, never a path taken.
        nextPane(context: context, forward: forward).flatMap(Pane.init(context:)) ?? self
    }

    /// The pane a context names, or `nil` for a context that is not one of
    /// the three — the conversation is inside the reading pane, and the
    /// composer, palette and settings lists are not panes Tab walks.
    public init?(context: UiContext) {
        switch context {
        case .sidebar: self = .sidebar
        case .list: self = .list
        case .reader: self = .reader
        default: return nil
        }
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
    /// The Settings window. Both frontends put settings in a window; ADR 0031
    /// is why, and why the model behind it is shared.
    public static let settings = "settings"
    /// The conversation pane's own four. They are commands like any other —
    /// bound, rebindable, in the menu — but what they act on is a fold this
    /// frontend is holding, so the boundary has nothing to do with them.
    /// Writing mail: the four verbs that open a compose window (#1272). The
    /// draft is the boundary's; the *window* is this frontend's, which is why
    /// these are handled here rather than dispatched.
    public static let compose = "compose"
    public static let reply = "reply"
    public static let replyAll = "reply_all"
    public static let forward = "forward"
    public static let toggleSidebar = "toggle_sidebar"
    /// The conversation rail, hidden or shown -- `⇧I`, the reader's choice
    /// for this window (FR-047). The rail is this frontend's to draw.
    public static let toggleRail = "toggle_rail"
    /// Paging the reading pane. Here rather than dispatched because the
    /// document is this frontend's — and because a hardened web view has no
    /// scroll call, so the jump between the shared anchors happens in the
    /// view. See `ReaderPaging`.
    public static let scrollReaderDown = "scroll_reader_down"
    public static let scrollReaderUp = "scroll_reader_up"
    /// The composer's own verbs, answered by the compose window that has the
    /// keyboard. The draft being written is in a window this frontend owns
    /// and the store has not seen most of it, which is why these stop here —
    /// `ComposeCommands` is the route from the id to the model.
    public static let composeVerbs = ComposeCommands.handled
    /// The sidebar's own keyboard. The folder tree, which rows are collapsed
    /// and where the keyboard is inside it are all this frontend's state, so
    /// a session has nothing to answer these with — `SidebarWalk` is the
    /// rule and `Engine` holds the two pieces of state it needs.
    public static let nextFolder = "next_folder"
    public static let prevFolder = "prev_folder"
    public static let toggleFolder = "toggle_folder"
    public static let goToInbox = "go_to_inbox"
    public static let goToDrafts = "go_to_drafts"
    public static let goToSent = "go_to_sent"
    public static let goToFlagged = "go_to_flagged"
    /// Where the keyboard is among the panes, and whether a message is drawn
    /// as its sender wrote it. Both are this frontend's state: there is no
    /// drill-in to close and nothing is remembered about an original past the
    /// view it was asked for in.
    public static let openMessage = "open_message"
    public static let prevView = "prev_view"
    public static let viewOriginal = "view_original"
    /// The parts panel. Opening it is a surface, walking it moves a cursor
    /// this side holds, and every verb on it needs a dialog or a launcher —
    /// so all eight stop here. `PartsPanel` is the surface and `PartsModel`
    /// the cursor; the tree, the labels and the safe filename are all the
    /// boundary's.
    /// Re-ask the query the other way round. Intercepted rather than sent,
    /// because it is the *list* that has to be told to redraw afterwards.
    public static let toggleResultOrder = "toggle_result_order"
    /// Saved searches. All five patch `config.toml`, which this side reads
    /// at the moment it acts; two of them ask a question first.
    public static let saveSearch = "save_search"
    public static let renameSavedSearch = "rename_saved_search"
    public static let deleteSavedSearch = "delete_saved_search"
    public static let moveSavedSearchUp = "move_saved_search_up"
    public static let moveSavedSearchDown = "move_saved_search_down"
    public static let openParts = "open_parts"
    public static let nextPart = "next_part"
    public static let prevPart = "prev_part"
    public static let savePart = "save_part"
    public static let saveAllParts = "save_all_parts"
    public static let openPartExternally = "open_part_externally"
    public static let openPart = "open_part"
    public static let renderPartOnce = "render_part_once"
    /// The settings window's account verbs. Every one acts on the row the
    /// keyboard is on, which is `SettingsAccounts`'s cursor and this side's
    /// alone; two of them need a sheet on top of that.
    public static let addAccount = "add_account"
    public static let editConfig = "edit_config"
    public static let toggleAccountEnabled = "toggle_account_enabled"
    public static let removeAccount = "remove_account"
    public static let updateCredential = "update_credential"
    public static let rebuildAccountIndex = "rebuild_account_index"
    public static let setDefaultAccount = "set_default_account"
    public static let expandAll = "expand_all"
    public static let toggleFold = "toggle_fold"
    public static let nextInConversation = "next_in_conversation"
    public static let prevInConversation = "prev_in_conversation"

    /// Every id above, for the test that checks they still exist.
    public static let all = [
        palette, cheatSheet, search, back, cyclePane, cyclePaneBack, focusSidebar, settings,
        toggleSidebar, expandAll, toggleFold, nextInConversation, prevInConversation,
        scrollReaderDown, scrollReaderUp,
        nextFolder, prevFolder, toggleFolder,
        goToInbox, goToDrafts, goToSent, goToFlagged,
        openMessage, prevView, viewOriginal,
        addAccount, editConfig, toggleAccountEnabled, removeAccount,
        updateCredential, rebuildAccountIndex, setDefaultAccount,
        toggleResultOrder, saveSearch, renameSavedSearch, deleteSavedSearch,
        moveSavedSearchUp, moveSavedSearchDown,
        openParts, nextPart, prevPart,
        savePart, saveAllParts, openPartExternally, openPart, renderPartOnce,
        compose, reply, replyAll, forward, toggleRail,
    ] + composeVerbs
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
