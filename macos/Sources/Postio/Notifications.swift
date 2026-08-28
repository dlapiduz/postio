import AppKit
import PostioKit
import UserNotifications

/// Posting a new-mail notification, and handling the click.
///
/// The *decision* — whether an arrival is worth interrupting somebody for and
/// what it may say — is `MailNotifier` in PostioKit, where it is pure and
/// tested. This is the half that talks to the system, and it is deliberately
/// almost empty: everything here needs an authorised bundle, which is exactly
/// what a test cannot have.
@MainActor
final class MailNotifications: NSObject, UNUserNotificationCenterDelegate {
    /// What a click should open. Set by the engine, called on the main actor.
    var open: ((Int64, Int64?) -> Void)?

    /// Whether permission has been asked for yet.
    ///
    /// Asked on the first arrival worth notifying about rather than at launch.
    /// macOS remembers a refusal permanently, and an application that asks in
    /// its first second — before the user has seen a single message — is
    /// asking someone with no reason to say yes.
    private var asked = false

    /// The notification centre, or `nil` when there is no bundle.
    ///
    /// `UNUserNotificationCenter.current()` traps in an unbundled process, and
    /// `swift run` from the package is exactly that. Returning `nil` keeps the
    /// development loop working with notifications simply absent.
    private var center: UNUserNotificationCenter? {
        guard Bundle.main.bundleIdentifier != nil else { return nil }
        return .current()
    }

    func start() {
        center?.delegate = self
    }

    /// Post `notification`, asking for permission first if this is the first.
    func post(_ notification: MailNotification) {
        guard let center else { return }
        let content = UNMutableNotificationContent()
        content.title = notification.title
        content.body = notification.body
        // Ids only. The visible text already carries everything this
        // notification says; anything extra riding along in `userInfo` would
        // be content leaving the application for the notification database,
        // which is not somewhere Postio's privacy rules reach.
        content.userInfo = [
            Self.mailboxKey: notification.mailbox,
            Self.messageKey: notification.message as Any,
        ]

        let request = UNNotificationRequest(
            // Stable per folder: this is what makes a second batch replace the
            // one on screen instead of stacking beside it.
            identifier: notification.identifier,
            content: content,
            trigger: nil
        )

        guard asked else {
            asked = true
            center.requestAuthorization(options: [.alert, .sound]) { granted, _ in
                guard granted else { return }
                center.add(request)
            }
            return
        }
        center.add(request)
    }

    // MARK: - UNUserNotificationCenterDelegate

    nonisolated func userNotificationCenter(
        _: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let info = response.notification.request.content.userInfo
        guard let mailbox = info[Self.mailboxKey] as? Int64 else { return }
        let message = info[Self.messageKey] as? Int64
        await MainActor.run {
            // Raise the window first: a click on a notification activates the
            // application whether or not any window had focus, and opening a
            // message behind another app is the same as doing nothing.
            NSApp.activate(ignoringOtherApps: true)
            open?(mailbox, message)
        }
    }

    // `nonisolated` because the delegate callback is: the keys are constants,
    // and making the callback hop to the main actor just to read two strings
    // would be an actor hop for nothing.
    nonisolated private static let mailboxKey = "mailbox"
    nonisolated private static let messageKey = "message"
}
