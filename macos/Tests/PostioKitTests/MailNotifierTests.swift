import Testing
@testable import PostioKit

/// When new mail is worth interrupting somebody for, and what it may say.
///
/// The decision is pure and the delivery is not, which is the only way this is
/// testable at all: `UNUserNotificationCenter` needs a bundled, authorised
/// application, and a test that needed one would prompt on a developer's
/// machine and hang every headless run — the same reason `MessageRowSource`
/// exists.
@Suite("New-mail notifications")
struct MailNotifierTests {
    private func arrival(mailbox: Int64 = 1, messages: [Int64] = [10]) -> MailArrival {
        MailArrival(account: 1, mailbox: mailbox, messages: messages)
    }

    private func decide(
        _ arrival: MailArrival,
        showing: Int64? = 99,
        active: Bool = false,
        name: String? = "Inbox"
    ) -> NotificationDecision {
        MailNotifier.decide(arrival, showing: showing, isActive: active, mailboxName: name)
    }

    @Test("an empty arrival is not a notification")
    func nothingArrived() {
        // The engine can emit a `NewMail` whose messages were all already
        // known. Interrupting somebody to say nothing happened is the worst
        // available outcome.
        #expect(decide(arrival(messages: [])) == .suppress(.nothingArrived))
    }

    @Test("mail landing in the folder on screen, in front of you, is not news")
    func alreadyOnScreen() {
        #expect(
            decide(arrival(mailbox: 5), showing: 5, active: true) == .suppress(.alreadyOnScreen)
        )
    }

    @Test("the same folder in a window you are not looking at still notifies")
    func sameFolderButNotFocused() {
        // "On screen" means both halves. A folder left open behind another
        // application is not something the user is watching, and this is the
        // case a naive "is this the open mailbox" check gets wrong.
        let decision = decide(arrival(mailbox: 5), showing: 5, active: false)
        #expect(decision != .suppress(.alreadyOnScreen))
    }

    @Test("mail landing elsewhere notifies even while the app is in front")
    func anotherFolderWhileActive() {
        let decision = decide(arrival(mailbox: 5), showing: 99, active: true)
        guard case .deliver = decision else {
            Issue.record("expected a notification, got \(decision)")
            return
        }
    }

    @Test("a burst is one notification carrying a count")
    func aBurstCounts() {
        guard case let .deliver(notification) = decide(arrival(messages: [1, 2, 3])) else {
            Issue.record("expected a notification")
            return
        }
        #expect(notification.body.contains("3"))
        #expect(notification.message == nil, "a burst has no one message to point at")
    }

    @Test("a single arrival names the message it is about")
    func aSingleArrivalPointsSomewhere() {
        guard case let .deliver(notification) = decide(arrival(messages: [42])) else {
            Issue.record("expected a notification")
            return
        }
        #expect(notification.message == 42, "clicking it has somewhere to land")
    }

    @Test("a second batch for one folder replaces the first rather than stacking")
    func oneNotificationPerFolder() {
        // The identifier is what `UNUserNotificationCenter` treats as "replace
        // what is showing". Several IDLE wake-ups in a row have to settle into
        // one notification saying the current total, not five popups.
        guard case let .deliver(first) = decide(arrival(mailbox: 7, messages: [1])),
              case let .deliver(second) = decide(arrival(mailbox: 7, messages: [2, 3]))
        else {
            Issue.record("expected two notifications")
            return
        }
        #expect(first.identifier == second.identifier)

        guard case let .deliver(other) = decide(arrival(mailbox: 8, messages: [4])) else {
            Issue.record("expected a notification")
            return
        }
        #expect(other.identifier != first.identifier, "a different folder is a different one")
    }

    @Test("the folder is named when known and not invented when not")
    func namesTheFolder() {
        guard case let .deliver(named) = decide(arrival(), name: "Archive"),
              case let .deliver(unnamed) = decide(arrival(), name: nil)
        else {
            Issue.record("expected notifications")
            return
        }
        #expect(named.title.contains("Archive"))
        #expect(!unnamed.title.isEmpty, "a folder we cannot name still notifies")
    }

    @Test("no message content reaches the notification")
    func carriesNoContent() {
        // `PRODUCT.md`: logs carry ids, counts and outcomes only, and a
        // notification is a log the lock screen reads out. The boundary helps
        // here — `UiEvent.newMail` carries ids and nothing else — and this
        // asserts nobody later adds a lookup to "improve" it.
        guard case let .deliver(notification) = decide(arrival(messages: [42])) else {
            Issue.record("expected a notification")
            return
        }
        let drawn = notification.title + " " + notification.body
        #expect(!drawn.contains("42"), "not even the id is drawn at somebody")
        #expect(notification.mailbox == 1)
    }
}
