import Observation
import PostioFFI

/// Which account row the settings window's keyboard is on, and what a
/// command has asked to be done to it (#1575).
///
/// The seven verbs in `Context::Accounts` all mean "do this to the row I am
/// looking at", so all seven need somewhere to read that row from. It was an
/// `@State` inside `SettingsPaneView` — which is why every one of them
/// resolved to nothing while the buttons beside them worked, the same defect
/// `OriginalView` and `RenderedOnce` record one window over.
///
/// **A missing cursor is a real answer.** Aiming at nothing does nothing;
/// falling back to "the first account" would remove somebody's mail on a
/// keystroke aimed at no row at all, which is the reasoning ADR 0005 Q6c
/// gives and the one `postio-gtk`'s `focused_account` follows.
@MainActor
@Observable
public final class SettingsAccounts {
    /// The account the keyboard is on, by id.
    public private(set) var cursor: Int64?

    /// What a command has asked the pane to put on screen.
    ///
    /// Only the two that need a surface. Enabling, defaulting, re-indexing
    /// and removing are calls the engine can make itself; adding an account
    /// and replacing a credential both need a sheet, and a command has no
    /// view to present one with.
    public enum Wish: Equatable, Sendable {
        case add
        case updateCredential(Int64)
    }

    public private(set) var wish: Wish?
    public private(set) var wishToken = 0

    public init() {}

    /// Put the keyboard on `account`, or on nothing.
    public func put(cursor: Int64?) {
        self.cursor = cursor
    }

    /// Ask the pane for `wish`.
    public func ask(_ wish: Wish) {
        self.wish = wish
        wishToken += 1
    }

    /// The row the keyboard is on, out of `accounts`.
    ///
    /// `nil` when the cursor names a row that is not there any more — a
    /// removal in one window while a command is pressed in another. The
    /// caller does nothing, which is the truthful outcome.
    public func focused(in accounts: [AccountFfi]) -> AccountFfi? {
        guard let cursor else { return nil }
        return accounts.first { $0.id == cursor }
    }
}
