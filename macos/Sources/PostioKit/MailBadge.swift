import Foundation
import UserNotifications

/// What System Settings says about Postio's badge, as far as it is known.
public enum SystemBadge: Equatable, Sendable {
    /// Notifications were never asked about, or the answer has not come
    /// back: the Dock draws a tile badge then, so `[sync] badge` decides.
    case unknown
    /// Notifications are allowed and so are badges.
    case allowed
    /// The Dock will not draw it: badges turned off in System Settings,
    /// authorised without `.badge`, or notifications refused outright.
    case turnedOff

    /// Read off `UNNotificationSettings`, as the Dock obeys it. Observed on
    /// macOS 26: a bundle whose notifications were refused gets no tile
    /// badge, and neither does one authorised without `.badge`; one never
    /// asked does. Drawing a label the Dock hides is harmless, but saying
    /// so here keeps the log honest about what a person sees.
    public init(status: UNAuthorizationStatus, badge: UNNotificationSetting) {
        switch status {
        case .notDetermined: self = .unknown
        case .denied: self = .turnedOff
        default: self = badge == .enabled ? .allowed : .turnedOff
        }
    }
}

/// The number on the Dock icon: Focus's unread inbox conversations, the
/// figure the header strip says ("312 · 41 unread"), across every account.
///
/// The figure is the engine's (`FocusStripFfi.unread`, on the strip's own
/// read); this decides only whether to draw it. `MailNotifications` applies
/// it to the Dock tile, which needs no notification permission.
public enum MailBadge {
    /// The Dock tile's label, or `nil` for a bare icon.
    ///
    /// `unread` is `nil` until the first count lands, and a guess would be
    /// a number that is wrong. Zero is a bare icon, as Mail draws it.
    public static func label(unread: UInt32?, enabled: Bool, system: SystemBadge) -> String? {
        guard enabled, system != .turnedOff, let unread, unread > 0 else { return nil }
        // No grouping separator: "1,204" is wider than the lozenge.
        return String(unread)
    }

    /// Whether every delivered notification should be withdrawn: with
    /// nothing unread, what they announced has all been seen.
    public static func withdrawsDelivered(unread: UInt32?) -> Bool {
        unread == 0
    }
}

/// A notification still in Notification Center: what `withdrawn` needs of it.
public struct DeliveredNotice: Equatable, Sendable {
    public let identifier: String
    /// The one message it is about, or `nil` for a batch.
    public let message: Int64?

    public init(identifier: String, message: Int64?) {
        self.identifier = identifier
        self.message = message
    }
}

extension MailNotifier {
    /// The delivered notifications that opening `opened` has answered: the
    /// ones about that message alone. A batch is left until the inbox has
    /// nothing unread.
    public static func withdrawn(_ delivered: [DeliveredNotice], opened: Int64) -> [String] {
        delivered.filter { $0.message == opened }.map(\.identifier)
    }

    /// Notification Center's group for an arrival at `account`: one stack
    /// per account, so two inboxes do not interleave.
    public static func thread(account: Int64) -> String {
        "account-\(account)"
    }
}
