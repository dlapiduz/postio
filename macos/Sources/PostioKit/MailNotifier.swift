import Foundation

/// New mail that arrived, as `UiEvent.newMail` reports it.
///
/// Ids and nothing else, which is the boundary's choice and the right one: a
/// notification is a log the lock screen reads out, and `PRODUCT.md`'s rule
/// that logs carry ids, counts and outcomes only applies here more than
/// anywhere. There is deliberately no lookup from here to a subject line.
public struct MailArrival: Equatable, Sendable {
    /// The account it arrived at.
    public let account: Int64
    /// The folder it landed in.
    public let mailbox: Int64
    /// The newly delivered messages.
    public let messages: [Int64]

    public init(account: Int64, mailbox: Int64, messages: [Int64]) {
        self.account = account
        self.mailbox = mailbox
        self.messages = messages
    }
}

/// Why an arrival did not become a notification.
public enum Suppressed: Equatable, Sendable {
    /// The event carried no new messages.
    case nothingArrived
    /// It landed in the folder the user is looking at, right now.
    case alreadyOnScreen
}

/// What to do about an arrival.
public enum NotificationDecision: Equatable, Sendable {
    case suppress(Suppressed)
    case deliver(MailNotification)
}

/// One notification, ready to hand `UNUserNotificationCenter`.
public struct MailNotification: Equatable, Sendable {
    /// Stable per folder, so a second batch **replaces** the one on screen
    /// rather than stacking beside it. Several `IDLE` wake-ups in a row have
    /// to settle into one notification saying the current total.
    public let identifier: String
    /// What it says. Counts and a folder name — never message content.
    public let title: String
    public let body: String
    /// Where a click should land.
    public let mailbox: Int64
    /// The one message a single arrival is about, if it is one.
    public let message: Int64?
}

/// Deciding whether new mail is worth interrupting somebody for.
///
/// The decision is pure so it can be asserted; delivery is not and is a thin
/// wrapper over it. `postio-app` does the same split for the same reason —
/// `gio::Notification` has no getters, `UNUserNotificationCenter` needs an
/// authorised bundle, and neither is something a test should need.
public enum MailNotifier {
    /// Whether `arrival` becomes a notification, and what it says.
    ///
    /// `showing` is the folder the list currently has open and `isActive` is
    /// whether Postio is the frontmost application. **Both** are required to
    /// suppress: a folder left open behind another application is not one the
    /// user is watching, and treating "this is the open mailbox" as sufficient
    /// is the version of this check that silently swallows the notification
    /// somebody actually needed.
    public static func decide(
        _ arrival: MailArrival,
        showing: Int64?,
        isActive: Bool,
        mailboxName: String?
    ) -> NotificationDecision {
        guard !arrival.messages.isEmpty else { return .suppress(.nothingArrived) }
        if isActive, showing == arrival.mailbox { return .suppress(.alreadyOnScreen) }

        let count = arrival.messages.count
        // A single arrival names its message so the click lands on that row. A
        // burst has no one message to point at — "3 new messages" does not
        // pick one — so it opens the folder instead, exactly as choosing it in
        // the sidebar would.
        let message = count == 1 ? arrival.messages.first : nil
        let title = mailboxName.map { "New mail in \($0)" } ?? "New mail"
        let body = count == 1 ? "1 new message" : "\(count) new messages"

        return .deliver(
            MailNotification(
                identifier: "new-mail-\(arrival.mailbox)",
                title: title,
                body: body,
                mailbox: arrival.mailbox,
                message: message
            )
        )
    }
}
