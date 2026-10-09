import PostioFFI

/// The toolbar's sync label (FR-010, contracts/mac-surfaces.md "Main
/// window"): the controller's words and the SF Symbol for its mark.
///
/// The words are `postio_ui::focus_state::sync_label`'s, said by the
/// controller as `FocusSyncLabel` (T096) -- "Synced 16:09", "Syncing
/// 12,408 of 18,204", "Offline", "Sync failed". Until that event existed
/// this type composed them itself, a Swift mirror of the Rust function;
/// now it only chooses a symbol for the mark.
public struct SyncLabel: Equatable, Sendable {
    public let text: String
    public let mark: SyncMarkFfi

    public init(text: String, mark: SyncMarkFfi) {
        self.text = text
        self.mark = mark
    }

    /// The SF Symbol standing for the mark: GTK's four icons, by meaning.
    public var symbol: String {
        switch mark {
        case .synced: return "checkmark.circle"
        case .syncing: return "arrow.triangle.2.circlepath"
        case .offline: return "wifi.slash"
        case .failed: return "exclamationmark.triangle"
        }
    }

    /// Drawn in red: an account's sync is failing. Being offline is not
    /// alarming -- everything local still works, and the banner says so.
    public var isAlarming: Bool { mark == .failed }
}
