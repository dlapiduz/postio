import Foundation
import PostioFFI

/// Ids and nothing else: a notification is a log the lock screen reads out, so there is
/// deliberately no lookup from here to a subject line.
public struct MailArrival: Equatable, Sendable {
    public let account: Int64
    public let mailbox: Int64
    public let messages: [Int64]

    public init(account: Int64, mailbox: Int64, messages: [Int64]) {
        self.account = account
        self.mailbox = mailbox
        self.messages = messages
    }
}

public enum Suppressed: Equatable, Sendable {
    case nothingArrived
    case alreadyOnScreen
    case notWatched
    case turnedOff
}

public enum NotificationDecision: Equatable, Sendable {
    case suppress(Suppressed)
    case deliver(MailNotification)
}

public struct MailNotification: Equatable, Sendable {
    /// Stable per folder, so a second batch replaces the one on screen: several IDLE wake-ups in a
    /// row settle into one notification saying the current total.
    public let identifier: String
    /// What it says. Counts and a folder name — never message content.
    public let title: String
    public let body: String
    public let mailbox: Int64
    public let message: Int64?
    public let thread: String
}

/// The decision is the engine's (`postio_ui::notify`), so this and the classic app cannot drift on
/// when to suppress or where a click lands.
/// Delivery is `MailNotifications`, because `UNUserNotificationCenter` needs an authorised bundle.
public enum MailNotifier {
    /// Both `showing` and `isActive` are required to suppress: a folder left open behind another
    /// application is not one the user is
    /// watching, and treating the open mailbox as sufficient swallows a notification somebody
    /// needed.
    public static func decide(
        _ arrival: MailArrival,
        showing: Int64?,
        isActive: Bool,
        mailboxName: String?,
        enabled: Bool = true,
        role: MailboxRoleFfi = .inbox,
        notifyRoles: [String] = ["inbox"]
    ) -> NotificationDecision {
        guard enabled else { return .suppress(.turnedOff) }
        let decision = decideNotification(
            arrival: MailArrivalFfi(mailbox: arrival.mailbox, messages: arrival.messages),
            showing: showing,
            active: isActive,
            mailboxName: mailboxName,
            role: role,
            notifyRoles: notifyRoles
        )
        switch decision {
        case let .suppress(reason):
            switch reason {
            case .nothingArrived: return .suppress(.nothingArrived)
            case .alreadyOnScreen: return .suppress(.alreadyOnScreen)
            case .notWatched: return .suppress(.notWatched)
            }
        case let .deliver(notification):
            return .deliver(
                MailNotification(
                    identifier: notification.identifier,
                    title: notification.title,
                    body: notification.body,
                    mailbox: notification.mailbox,
                    message: notification.message,
                    thread: thread(account: arrival.account)
                )
            )
        }
    }
}
