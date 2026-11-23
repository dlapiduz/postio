import PostioFFI

public enum KeyWindow: Equatable, Sendable, CaseIterable {
    case main
    case compose
    case settings
    case message
}

/// The window decides first, and a pane only gets a say when the keyboard is in the window that has
/// panes. The monitor sees every key press including compose windows; asking only the main window's
/// pane meant composer-only bindings never resolved, and `a` in a compose window archived whatever
/// the list cursor was on.
public enum KeyboardContext {
    public static func resolving(keyWindow: KeyWindow, mainWindow: UiContext) -> UiContext {
        switch keyWindow {
        case .main:
            return mainWindow
        case .compose:
            return .composer
        case .settings:
            // The list's verbs must not resolve here, so `d` in settings removes nothing from a
            // mailbox.
            return .accounts
        case .message:
            return .reader
        }
    }
}
