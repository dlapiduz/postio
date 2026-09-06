import PostioFFI

/// How an account reads in the settings pane.
///
/// Small on purpose. Everything here is a *presentation* decision that the
/// boundary did not already make: joining the facts, choosing the tag, and
/// what an empty list says. The facts themselves are worded on the Rust side
/// (`postio_ui::account`), because two panes assembling their own would be
/// two descriptions of one account.
public enum AccountRow {
    /// The `·`-joined line under the address.
    public static func line(_ account: AccountFfi) -> String {
        account.facts.joined(separator: " · ")
    }

    /// The tag beside the address, or `nil` when there is nothing to say.
    ///
    /// "default" rather than "primary": it says what the marker *does* — new
    /// messages come from this account — instead of asserting that this one
    /// matters more, which is #960's fence.
    public static func tag(_ account: AccountFfi) -> String? {
        account.isDefault ? "default" : nil
    }

    /// Whether this account needs somebody to do something about it.
    ///
    /// An expired token is the case the canvas draws: the account is there,
    /// the mail is there, and nothing will arrive until somebody signs in
    /// again. Read off the facts the boundary already words rather than
    /// re-derived, so "expired" means the same thing in both frontends.
    public static func needsAttention(_ account: AccountFfi) -> Bool {
        account.facts.contains { fact in
            let fact = fact.lowercased()
            return fact.contains("expired") || fact.contains("reconnect")
        }
    }

    /// What the pane says when there are no accounts.
    ///
    /// Canvas 3d: never a shrug — and now it can point at the button that
    /// does it, which is what changed with #1279.
    public static let emptyMessage =
        "No accounts yet. Add one with the + button below."
}
