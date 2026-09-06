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

    /// What the pane says when there are no accounts.
    ///
    /// Canvas 3d: never a shrug. On macOS this is also the ordinary case, so
    /// it names the way in that exists rather than pointing at an Add button
    /// this build does not have yet (#649).
    public static let emptyMessage =
        "No accounts yet. Adding one from here is not built on macOS; "
        + "`cargo run -p postio-session --bin postio-provision` sets one up."
}
