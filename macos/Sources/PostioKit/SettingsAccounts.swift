import Observation
import PostioFFI

/// Every `Context::Accounts` verb means "do this to the row I am looking at", so the row needs
/// somewhere to be read from other than view `@State`, which commands cannot reach (#1575). A
/// missing cursor is a real answer: falling back to the first account would remove someone's mail
/// on a keystroke aimed at no row (ADR 0005 Q6c).
@MainActor
@Observable
public final class SettingsAccounts {
    public private(set) var cursor: Int64?

    /// Only what needs a surface: adding an account and replacing a credential need a sheet; the
    /// rest the engine does itself.
    public enum Wish: Equatable, Sendable {
        case add
        case updateCredential(Int64)
    }

    public private(set) var wish: Wish?
    public private(set) var wishToken = 0

    public init() {}

    public func put(cursor: Int64?) {
        self.cursor = cursor
    }

    public func ask(_ wish: Wish) {
        self.wish = wish
        wishToken += 1
    }

    /// Taken both when the token moves and when the pane appears, since a command that opens the
    /// window creates the pane after the token moved; cleared so the sheet does not reopen each
    /// time.
    public func take() -> Wish? {
        defer { wish = nil }
        return wish
    }

    /// `nil` when the cursor names a row no longer there (removed in another window); the caller
    /// does nothing.
    public func focused(in accounts: [AccountFfi]) -> AccountFfi? {
        guard let cursor else { return nil }
        return accounts.first { $0.id == cursor }
    }
}
