import PostioFFI

/// Which window has the keyboard, as far as resolving a key cares.
///
/// Not every `NSWindow` — three roles, because the registry's contexts are
/// about surfaces rather than about panels. Anything Postio opens that is not
/// the main window is one of the other two.
public enum KeyWindow: Equatable, Sendable, CaseIterable {
    /// The three-pane window: list, sidebar, reading pane, and the overlays
    /// drawn over them.
    case main
    /// A compose window. Several can be open at once; they resolve alike.
    case compose
    /// The settings window.
    case settings
}

/// Which surface a keystroke resolves as.
///
/// # Why this is not just the focused pane
///
/// `KeyMonitor` installs a **local** monitor, which sees every key press in
/// the application — including the ones typed into a compose window. What it
/// asked for the context was the main window's focused pane, so
/// `UiContext.composer` was never once the answer. Two things followed.
///
/// Every composer-only binding resolved to nothing: `send`, `save_draft`,
/// `discard_draft`, `attach_file`, `bold`, `italic`, `insert_link` are
/// available in `Context::Composer` and the resolver was never told it was
/// there. And the main window's bindings kept resolving while a message was
/// being written, so `a` in a compose window archived whatever the list
/// cursor was on — a destructive verb, reached by typing a letter into a
/// text field, in a window that has nothing to archive.
///
/// So the **window decides first**, and a pane only gets a say when the
/// keyboard is in the window that has panes. That is also the rule
/// `Context::Composer`'s own doc comment states from the other side: the
/// composer is one context whether it is the reading pane or a window of its
/// own, "so a binding that works in the pane works in the detached window
/// without a second context to keep in step".
public enum KeyboardContext {
    /// The context `keyWindow` resolves keys as, given what the main window's
    /// focus was.
    public static func resolving(keyWindow: KeyWindow, mainWindow: UiContext) -> UiContext {
        switch keyWindow {
        case .main:
            // The pane, or whichever overlay the engine has put up — the
            // palette and the search bar are drawn inside this window and
            // set the context themselves.
            return mainWindow
        case .compose:
            return .composer
        case .settings:
            // `Context::Accounts` is the surface inside the settings window
            // that has keys of its own. What matters most is the negative:
            // the list's verbs must not resolve here, so `d` in settings
            // removes nothing from a mailbox.
            return .accounts
        }
    }
}
