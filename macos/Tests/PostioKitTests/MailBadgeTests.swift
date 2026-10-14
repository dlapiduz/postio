import Testing
import UserNotifications
@testable import PostioKit

/// The number on the Dock icon, and what Notification Center keeps once the
/// mail it announced has been dealt with.
///
/// The Dock tile and `UNUserNotificationCenter` need a bundled application,
/// so the rule lives here, pure, and `MailNotifications` only applies it.
@Suite("The unread badge")
struct MailBadgeTests {
    @Test("the badge is the strip's unread figure, as a plain number")
    func theUnreadFigure() {
        #expect(MailBadge.label(unread: 7, enabled: true, system: .allowed) == "7")
        // No grouping separator: "1,204" is wider than the tile's lozenge,
        // and Mail draws the bare number.
        #expect(MailBadge.label(unread: 1204, enabled: true, system: .allowed) == "1204")
    }

    @Test("nothing unread clears the badge rather than drawing a zero")
    func zeroClears() {
        #expect(MailBadge.label(unread: 0, enabled: true, system: .allowed) == nil)
    }

    @Test("before the counts arrive there is no badge, not a guess")
    func unknownClears() {
        #expect(MailBadge.label(unread: nil, enabled: true, system: .allowed) == nil)
    }

    @Test("[sync] badge = false keeps the icon bare")
    func switchedOff() {
        #expect(MailBadge.label(unread: 7, enabled: false, system: .allowed) == nil)
    }

    @Test("badges turned off in System Settings are honoured")
    func systemTurnedOff() {
        #expect(MailBadge.label(unread: 7, enabled: true, system: .turnedOff) == nil)
    }

    @Test("before permission is asked, the count is drawn")
    func notAskedStillCounts() {
        // The Dock tile needs no permission until there is an answer, and
        // Postio asks only when the first notification is worth posting.
        #expect(MailBadge.label(unread: 7, enabled: true, system: .unknown) == "7")
    }

    @Test("what System Settings says, read as the Dock obeys it")
    func whatTheSystemSays() {
        // Observed on macOS 26: the Dock draws no tile badge for a bundle
        // whose notifications were refused, nor for one authorised without
        // `.badge`; it draws one for a bundle never asked.
        #expect(SystemBadge(status: .notDetermined, badge: .notSupported) == .unknown)
        #expect(SystemBadge(status: .authorized, badge: .enabled) == .allowed)
        #expect(SystemBadge(status: .authorized, badge: .disabled) == .turnedOff)
        #expect(SystemBadge(status: .authorized, badge: .notSupported) == .turnedOff)
        #expect(SystemBadge(status: .denied, badge: .enabled) == .turnedOff)
        #expect(SystemBadge(status: .provisional, badge: .enabled) == .allowed)
    }

    @Test("an inbox with nothing unread withdraws what was delivered")
    func zeroWithdrawsEverything() {
        #expect(MailBadge.withdrawsDelivered(unread: 0))
        #expect(!MailBadge.withdrawsDelivered(unread: 3))
        #expect(!MailBadge.withdrawsDelivered(unread: nil))
    }
}

@Suite("Notification Center housekeeping")
struct NotificationHousekeepingTests {
    @Test("notifications group by account")
    func groupedByAccount() {
        let arrival = MailArrival(account: 4, mailbox: 1, messages: [10])
        guard case let .deliver(notification) = MailNotifier.decide(
            arrival, showing: nil, isActive: false, mailboxName: "Inbox", enabled: true)
        else {
            Issue.record("an arrival in the background notifies")
            return
        }
        #expect(notification.thread == "account-4")
        let other = MailArrival(account: 5, mailbox: 2, messages: [11])
        guard case let .deliver(second) = MailNotifier.decide(
            other, showing: nil, isActive: false, mailboxName: "Inbox", enabled: true)
        else { return }
        #expect(second.thread != notification.thread)
    }

    @Test("[sync] notify = false posts nothing")
    func turnedOff() {
        let arrival = MailArrival(account: 1, mailbox: 1, messages: [10])
        #expect(
            MailNotifier.decide(
                arrival, showing: nil, isActive: false, mailboxName: "Inbox", enabled: false)
                == .suppress(.turnedOff))
    }

    @Test("opening a message withdraws the notification about it, and only that one")
    func openingWithdraws() {
        let delivered = [
            DeliveredNotice(identifier: "mailbox-1", message: 10),
            DeliveredNotice(identifier: "mailbox-2", message: 11),
            // A batch: about several messages, so opening one of them does
            // not answer it.
            DeliveredNotice(identifier: "mailbox-3", message: nil),
        ]
        #expect(MailNotifier.withdrawn(delivered, opened: 10) == ["mailbox-1"])
        #expect(MailNotifier.withdrawn(delivered, opened: 99) == [])
    }
}
