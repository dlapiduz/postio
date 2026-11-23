import PostioFFI

/// Words are the controller's (`FocusSyncLabel`); this only chooses a symbol for the mark.
public struct SyncLabel: Equatable, Sendable {
    public let text: String
    public let mark: SyncMarkFfi

    public init(text: String, mark: SyncMarkFfi) {
        self.text = text
        self.mark = mark
    }

    public var symbol: String {
        switch mark {
        case .synced: return "checkmark.circle"
        case .syncing: return "arrow.triangle.2.circlepath"
        case .offline: return "wifi.slash"
        case .failed: return "exclamationmark.triangle"
        }
    }

    /// Offline is not alarming: everything local still works, and the banner says so.
    public var isAlarming: Bool { mark == .failed }
}
