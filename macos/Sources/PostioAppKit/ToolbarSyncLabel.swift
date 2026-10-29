import PostioKit
import SwiftUI

/// The sync label: a symbol and the controller's words ("Synced 16:09",
/// "Syncing 12,408 of 18,204", "Offline", "Sync failed"), as
/// `FocusSyncLabel` last said them.
public struct ToolbarSyncLabel: View {
    let label: SyncLabel

    /// Before the symbol: from macOS 26 the toolbar puts the label and the
    /// search field on one glass capsule, and the label starts it, so its
    /// symbol sat on the capsule's rounded end.
    public static let leading: CGFloat = 10
    /// After the words, before the field.
    public static let trailing: CGFloat = 4

    public init(label: SyncLabel) { self.label = label }

    public var body: some View {
        Words(label: label)
            .padding(.leading, Self.leading)
            .padding(.trailing, Self.trailing)
            .fixedSize()
    }

    /// The symbol and the words, as wide as they are.
    struct Words: View {
        let label: SyncLabel

        var body: some View {
            HStack(spacing: 5) {
                Image(systemName: label.symbol).font(.system(size: 11))
                Text(label.text).font(.system(size: 12).monospacedDigit())
            }
            .foregroundStyle(label.isAlarming ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
            .fixedSize()
            .accessibilityElement(children: .combine)
        }
    }
}
