import AppKit
import PostioAppKit
import PostioKit
import UserNotifications
import os

/// The half that talks to the system; the decision is `MailNotifier` in PostioKit. Everything here
/// needs an authorised bundle, which a test cannot have.
@MainActor
final class MailNotifications: NSObject, UNUserNotificationCenterDelegate {
    var open: ((Int64, Int64?) -> Void)?

    /// Asked on the first arrival worth notifying about, not at launch: macOS remembers a refusal
    /// permanently.
    private var asked = false

    /// `nil` when there is no bundle: `UNUserNotificationCenter.current()` traps in an unbundled
    /// process (`swift run`).
    private var center: UNUserNotificationCenter? {
        guard Bundle.main.bundleIdentifier != nil else { return nil }
        return .current()
    }

    func start() {
        center?.delegate = self
        // Read at launch and whenever Postio returns to the front: the person may turn badges off
        // in System Settings.
        offerTheBadge()
        NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.readSystemBadge() }
        }
    }

    private var unread: UInt32?
    private var badgeOn = true
    private var system = SystemBadge.unknown

    private static let log = Logger(subsystem: "dev.postio.Postio", category: "notifications")

    func unread(_ unread: UInt32?, badge: Bool) {
        let withdraw = MailBadge.withdrawsDelivered(unread: unread) && self.unread != 0
        self.unread = unread
        badgeOn = badge
        drawBadge()
        if withdraw { center?.removeAllDeliveredNotifications() }
    }

    /// Needs no notification permission. `setBadgeCount` draws on this same tile and needs
    /// permission,
    /// so it is not a second writer.
    private func drawBadge() {
        let label = MailBadge.label(unread: unread, enabled: badgeOn, system: system)
        guard NSApp.dockTile.badgeLabel != label else { return }
        NSApp.dockTile.badgeLabel = label
        // A count, never content.
        Self.log.debug("dock badge \(label ?? "cleared", privacy: .public)")
    }

    /// Earlier builds asked for alerts and sounds only, and the Dock draws no badge without
    /// `.badge`.
    /// Asking again after the person has answered shows no prompt and registers the option; before
    /// an answer this asks nothing.
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

    func post(_ notification: MailNotification) {
        guard let center else { return }
        let content = UNMutableNotificationContent()
        content.title = notification.title
        content.body = notification.body
        content.threadIdentifier = notification.thread
        // Ids only: anything extra in `userInfo` would be content leaving for the notification
        // database.
        content.userInfo = [
            Self.mailboxKey: notification.mailbox,
            Self.messageKey: notification.message as Any,
        ]

        let request = UNNotificationRequest(
            // Stable per folder, so a second batch replaces the one on screen.
            identifier: notification.identifier,
            content: content,
            trigger: nil
        )

        guard asked else {
            asked = true
            // `.badge` too, so System Settings offers its badge switch, which `readSystemBadge`
            // honours.
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

    nonisolated func userNotificationCenter(
        _: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let info = response.notification.request.content.userInfo
        guard let mailbox = info[Self.mailboxKey] as? Int64 else { return }
        let message = info[Self.messageKey] as? Int64
        await MainActor.run {
            // Raise the window first: opening a message behind another app is the same as doing
            // nothing.
            NSApp.activate(ignoringOtherApps: true)
            open?(mailbox, message)
        }
    }

    // `nonisolated` because the delegate callback is; hopping to the main actor to read constants
    // is waste.
    nonisolated private static let mailboxKey = "mailbox"
    nonisolated private static let messageKey = "message"
}
