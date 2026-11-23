import PostioKit
import SwiftUI

public struct ToolbarSyncLabel: View {
    let label: SyncLabel

    /// From macOS 26 the toolbar puts the label and search field on one glass capsule that the
    /// label starts, so its symbol sat on the rounded end.
    public static let leading: CGFloat = 10
    public static let trailing: CGFloat = 4

    public init(label: SyncLabel) { self.label = label }

    public var body: some View {
        Words(label: label)
            .padding(.leading, Self.leading)
            .padding(.trailing, Self.trailing)
            .fixedSize()
    }

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
