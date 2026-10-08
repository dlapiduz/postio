import Foundation

/// What the toolbar's sync label says (FR-010, contracts/mac-surfaces.md
/// "Main window"), and the symbol beside it.
///
/// **An FFI gap, mirrored.** These are `postio_ui::focus_state::sync_label`'s
/// words, in its order: a failure outranks being offline, offline outranks
/// a pass that cannot be running, a pass in flight outranks the time of the
/// last one. The boundary does not export it yet; when it does, this type
/// becomes the call.
public struct SyncLabel: Equatable, Sendable {
    public let text: String
    /// An SF Symbol standing for the same state.
    public let symbol: String

    public init(
        offline: Bool,
        failing: Bool,
        syncing: (done: UInt32, total: UInt32)?,
        lastSynced: Int64?,
        zone: TimeZone = .current
    ) {
        if failing {
            (text, symbol) = ("Sync failed", "exclamationmark.triangle")
        } else if offline {
            (text, symbol) = ("Offline", "wifi.slash")
        } else if let syncing {
            let words = "Syncing \(Counted.grouped(syncing.done)) of \(Counted.grouped(syncing.total))"
            (text, symbol) = (words, "arrow.triangle.2.circlepath")
        } else if let lastSynced {
            (text, symbol) = ("Synced \(Self.clock(lastSynced, zone))", "checkmark.circle")
        } else {
            (text, symbol) = ("Not synced yet", "arrow.triangle.2.circlepath")
        }
    }

    /// `%H:%M` in `zone`, as `postio_ui` writes it.
    private static func clock(_ seconds: Int64, _ zone: TimeZone) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = zone
        let parts = calendar.dateComponents(
            [.hour, .minute], from: Date(timeIntervalSince1970: TimeInterval(seconds)))
        return String(format: "%02d:%02d", parts.hour ?? 0, parts.minute ?? 0)
    }
}

/// A count with its thousands grouped by commas, as `postio_ui`'s
/// `selection::count` writes it: "50000" is not a number anybody reads.
/// Always commas, whatever the locale, so the two apps say the same thing.
enum Counted {
    static func grouped(_ value: UInt32) -> String {
        let digits = String(value)
        var out = ""
        for (index, digit) in digits.enumerated() {
            if index > 0, (digits.count - index) % 3 == 0 { out.append(",") }
            out.append(digit)
        }
        return out
    }
}
