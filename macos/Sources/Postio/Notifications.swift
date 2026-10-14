import AppKit
import PostioAppKit
import PostioKit
import UserNotifications
import os

/// Posting a new-mail notification, handling the click, and the Dock's
/// unread count.
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
        // The person may have turned badges off in System Settings while
        // Postio was not looking; read it now and whenever Postio comes back
        // to the front, which is where somebody returns from System Settings.
        offerTheBadge()
        NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.readSystemBadge() }
        }
    }

    // MARK: the Dock's count

    /// The inbox's unread figure, as the strip last said it.
    private var unread: UInt32?
    /// `[sync] badge`.
    private var badgeOn = true
    /// What System Settings says about badges.
    private var system = SystemBadge.unknown

    private static let log = Logger(subsystem: "dev.postio.Postio", category: "notifications")

    /// The strip's unread figure and `[sync] badge`, as they now stand:
    /// draw the Dock's count, and withdraw what was delivered once there is
    /// nothing left unread.
    func unread(_ unread: UInt32?, badge: Bool) {
        let withdraw = MailBadge.withdrawsDelivered(unread: unread) && self.unread != 0
        self.unread = unread
        badgeOn = badge
        drawBadge()
        if withdraw { center?.removeAllDeliveredNotifications() }
    }

    /// The Dock tile's label: it needs no notification permission, so the
    /// count is there for somebody who refused the banners too. Notification
    /// Center's own `setBadgeCount` draws on this same tile, and would only
    /// work with permission, so it is not used as a second writer.
    private func drawBadge() {
        let label = MailBadge.label(unread: unread, enabled: badgeOn, system: system)
        guard NSApp.dockTile.badgeLabel != label else { return }
        NSApp.dockTile.badgeLabel = label
        // A count, never content.
        Self.log.debug("dock badge \(label ?? "cleared", privacy: .public)")
    }

    /// Postio once asked for alerts and sounds only, and the Dock draws no
    /// tile badge for a bundle authorised without `.badge`. Asking again
    /// once the person has answered shows no prompt; it registers the
    /// option, so System Settings offers its Badges switch. Before there is
    /// an answer this asks nothing: the first question waits for the first
    /// notification worth posting.
    private func offerTheBadge() {
        guard let center else { return }
        Task { @MainActor in
            let settings = await center.notificationSettings()
            if settings.authorizationStatus != .notDetermined {
                asked = true
                _ = try? await center.requestAuthorization(options: [.alert, .sound, .badge])
            }
            readSystemBadge()
        }
    }

    private func readSystemBadge() {
        guard let center else { return }
        Task { @MainActor in
            let settings = await center.notificationSettings()
            system = SystemBadge(status: settings.authorizationStatus, badge: settings.badgeSetting)
            drawBadge()
        }
    }

    /// The message `message` was opened: withdraw a notification about it
    /// alone. One about a batch stays until nothing is unread.
    func opened(_ message: Int64) {
        guard let center else { return }
        Task { @MainActor in
            let delivered = await center.deliveredNotifications().map {
                DeliveredNotice(
                    identifier: $0.request.identifier,
                    message: $0.request.content.userInfo[Self.messageKey] as? Int64)
            }
            let gone = MailNotifier.withdrawn(delivered, opened: message)
            if !gone.isEmpty { center.removeDeliveredNotifications(withIdentifiers: gone) }
        }
    }

    /// Post `notification`, asking for permission first if this is the first.
    func post(_ notification: MailNotification) {
        guard let center else { return }
        let content = UNMutableNotificationContent()
        content.title = notification.title
        content.body = notification.body
        // One stack per account in Notification Center.
        content.threadIdentifier = notification.thread
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
            // `.badge` too, so System Settings offers its "Badge application
            // icon" switch for Postio, which `readSystemBadge` then honours.
            Task { @MainActor in
                let granted = (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
                readSystemBadge()
                guard granted else { return }
                try? await center.add(request)
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
