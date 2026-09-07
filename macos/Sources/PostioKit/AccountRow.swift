import Foundation
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
    ///
    /// `mailboxes` adds what the canvas draws and the boundary cannot say
    /// cheaply: how much mail this account has. It is summed from the folder
    /// counts the window already holds rather than counted again — the
    /// sidebar has them, they are cached on the mailbox rows, and asking the
    /// store for a second opinion on every settings open would be a scan for
    /// a line nobody is waiting on.
    ///
    /// `weight` is what the mail takes on disk, worded by
    /// `postio_ui::format::mail_weight` so both frontends describe a store
    /// the same way. `nil` when there is nothing to weigh — a fresh account
    /// says nothing rather than `0 B`, which reads as a failure.
    public static func line(
        _ account: AccountFfi,
        mailboxes: [MailboxFfi] = [],
        weight: String? = nil
    ) -> String {
        var facts = account.facts
        let messages = mailboxes
            .filter { $0.account == account.id }
            .reduce(0) { $0 + Int($1.total) }
        if messages > 0 {
            facts.append("\(formatted(messages)) msg")
        }
        if let weight, !weight.isEmpty {
            facts.append(weight)
        }
        return facts.joined(separator: " · ")
    }

    /// `4291` as `4,291` — a five-digit number is read wrong without it.
    private static func formatted(_ count: Int) -> String {
        count.formatted(.number.grouping(.automatic))
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

    /// Whether a removal is safe to offer for this account.
    ///
    /// Always, deliberately: the *confirmation* is what makes it safe, not a
    /// disabled button. What must never happen is a removal that leaves the
    /// credential behind, and that is `postio_session::checkup`'s job.
    public static func canRemove(_ account: AccountFfi) -> Bool {
        account.id > 0
    }
}
