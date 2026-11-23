import Foundation
import PostioFFI

public enum AccountRow {
    /// Summed from the folder counts the window already holds rather than asking the store for a
    /// second opinion.
    /// A `nil` weight says nothing rather than `0 B`, which reads as a failure.
    public static func line(
        _ account: AccountFfi,
        mailboxes: [MailboxFfi] = [],
        weight: String? = nil
    ) -> String {
        var facts = account.facts
        // Folders only: Flagged and Snoozed are views over mail already counted where it lives.
        let messages = mailboxes
            .filter { $0.account == account.id && !$0.isView }
            .reduce(0) { $0 + Int($1.total) }
        if messages > 0 {
            facts.append("\(formatted(messages)) msg")
        }
        if let weight, !weight.isEmpty {
            facts.append(weight)
        }
        return facts.joined(separator: " · ")
    }

    private static func formatted(_ count: Int) -> String {
        count.formatted(.number.grouping(.automatic))
    }

    /// "default" rather than "primary": it says what the marker does without asserting this account
    /// matters more (#960).
    public static func tag(_ account: AccountFfi) -> String? {
        account.isDefault ? "default" : nil
    }

    /// The boundary's answer, not a word scanned out of the fact line: that scan never matched, so
    /// the warning and Reconnect button were dead code (#1584).
    public static func needsAttention(_ account: AccountFfi) -> Bool {
        account.needsAttention
    }

    public static let emptyMessage =
        "No accounts yet. Add one with the + button below."

    /// Always: the confirmation makes removal safe, not a disabled button. Leaving no credential
    /// behind is `postio_session::checkup`'s job.
    public static func canRemove(_ account: AccountFfi) -> Bool {
        account.id > 0
    }
}

extension MailboxFfi {
    /// A view has nothing to `SELECT` or sync, and counting one counts its mail twice.
    public var isView: Bool { id == 0 }
}
