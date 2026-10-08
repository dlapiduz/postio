import PostioFFI

// What a Focus row says to VoiceOver is `FocusRowModel.accessibilityLabel`,
// next to the row it describes; the classic list's `Announcements` went with
// that list (specs/009-focus-macos T034).

/// The commands this frontend presents a surface for, rather than sending on.
///
/// Almost everything goes to `invoke`, where the boundary decides whether it
/// is its own or the engine's. These are the exceptions: each one *is* a
/// window, and a session cannot present one. The classic app's `run_action` made
/// the same call for the same reason.
///
/// They are named here rather than written as literals at the `switch`,
/// because a literal that no longer matches the registry is a key that
/// silently does nothing — `/` not opening search, with no error anywhere to
/// say why — and `everyInterceptedCommandIsInTheRegistry` is what notices.
/// Keeping the list short is what stops it becoming the hand-maintained
/// command table #657 exists to prevent.
public enum Intercepted {
    // Not `/`, ⌘K, `g o` or the go-to keys: Focus's controller opens the
    // command bar and the folders popover and goes to each place, and this
    // frontend draws what it says (specs/009-focus-macos T085, T086). Not
    // `?` either: the controller opens and closes the key map (T106).
    public static let back = "back"
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
    /// Where the keyboard is among the panes, and whether a message is drawn
    /// as its sender wrote it. Both are this frontend's state: there is no
    /// drill-in to close and nothing is remembered about an original past the
    /// view it was asked for in.
    public static let openMessage = "open_message"
    public static let prevView = "prev_view"
    public static let viewOriginal = "view_original"
    /// `⇧⌘O`: reader view for one message (spec 006 FR-031).
    public static let toggleReaderView = "toggle_reader_view"
    /// The reader's zoom (spec 006 FR-021): `⌘+`, `⌘-`, `⌘0`.
    public static let zoomIn = "zoom_in"
    public static let zoomOut = "zoom_out"
    public static let zoomReset = "zoom_reset"
    /// Find in the open message or conversation (spec 006 FR-018).
    public static let findInMessage = "find_in_message"
    public static let findNext = "find_next"
    public static let findPrevious = "find_previous"
    /// The reader's `i i`: the one-view render the notice's Show button runs, so
    /// the key, the palette row and the notice's Show button cannot drift.
    public static let showImages = "show_images"
    /// The palette's "Quit Postio"; AppKit answers `⌘Q` from the menu itself.
    public static let quit = "quit"
    /// The reader's `X`: the unsubscribe banner's button, from the keyboard
    /// and the palette, and only where the banner is.
    public static let unsubscribe = "unsubscribe"
    /// The reader's `i a`: the blocked-images notice's "Always allow", for
    /// the sender the notice names, and only where the notice is.
    public static let alwaysShowImages = "always_show_images"
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
        back, settings,
        expandAll, toggleFold, nextInConversation, prevInConversation,
        scrollReaderDown, scrollReaderUp,
        openMessage, prevView, viewOriginal, toggleReaderView, zoomIn, zoomOut, zoomReset,
        findInMessage, findNext, findPrevious,
        addAccount, editConfig, toggleAccountEnabled, removeAccount,
        updateCredential, rebuildAccountIndex, setDefaultAccount,
        showImages,
        quit, unsubscribe, alwaysShowImages,
        compose, reply, replyAll, forward,
    ] + composeVerbs
}
