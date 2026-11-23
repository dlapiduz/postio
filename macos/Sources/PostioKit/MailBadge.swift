import Foundation
import UserNotifications

public enum SystemBadge: Equatable, Sendable {
    case unknown
    case allowed
    case turnedOff

    /// Observed on macOS 26: a bundle whose notifications were refused gets no tile badge, nor does
    /// one authorised without `.badge`; one never asked does.
    public init(status: UNAuthorizationStatus, badge: UNNotificationSetting) {
        switch status {
        case .notDetermined: self = .unknown
        case .denied: self = .turnedOff
        default: self = badge == .enabled ? .allowed : .turnedOff
        }
    }
}

public enum MailBadge {
    /// `unread` is `nil` until the first count lands, and a guess would be wrong. Zero is a bare
    /// icon, as Mail draws it.
    public static func label(unread: UInt32?, enabled: Bool, system: SystemBadge) -> String? {
        guard enabled, system != .turnedOff, let unread, unread > 0 else { return nil }
        // No grouping separator: "1,204" is wider than the lozenge.
        return String(unread)
    }

    public static func withdrawsDelivered(unread: UInt32?) -> Bool {
        unread == 0
    }
}

public struct DeliveredNotice: Equatable, Sendable {
    public let identifier: String
    public let message: Int64?

    public init(identifier: String, message: Int64?) {
        self.identifier = identifier
        self.message = message
    }
}

extension MailNotifier {
    public static func withdrawn(_ delivered: [DeliveredNotice], opened: Int64) -> [String] {
        delivered.filter { $0.message == opened }.map(\.identifier)
    }

    public static func thread(account: Int64) -> String {
        "account-\(account)"
    }
}
