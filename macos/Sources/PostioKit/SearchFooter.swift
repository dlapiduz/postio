import SwiftUI

/// Every word and key is the controller's (`FocusResults`); the checked count's words are
/// `postio-ui`'s, handed in as `checked`.
public struct SearchFooter: View {
    let hints: [ResultsModel.Hint]
    let right: String
    let checked: String?
    let bulk: [ResultsModel.Hint]
    let selectAll: ResultsModel.Hint?

    public init(
        hints: [ResultsModel.Hint], right: String, checked: String?, bulk: [ResultsModel.Hint],
        selectAll: ResultsModel.Hint? = nil
    ) {
        self.hints = hints
        self.right = right
        self.checked = checked
        self.bulk = bulk
        self.selectAll = selectAll
    }

    public static let height: CGFloat = 32

    public var body: some View {
        HStack(spacing: 14) {
            if let checked {
                Text(checked).font(.system(size: 12, weight: .bold)).foregroundStyle(.primary)
                HStack(spacing: 14) {
                    ForEach(bulk) { hint in
                        HStack(spacing: 5) {
                            Text(hint.label).font(.system(size: 12, weight: .semibold)).foregroundStyle(.primary)
                            KeyCap(hint.cap)
                        }
                    }
                }
                Spacer(minLength: 8)
                if let selectAll {
                    HStack(spacing: 5) {
                        KeyCap(selectAll.cap)
                        Text(selectAll.label).font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                    .fixedSize()
                }
            } else {
                FocusHintRow(hints.map { FocusHintRow.Hint(cap: $0.cap, label: $0.label) })
                Text(right).font(.system(size: 12).monospacedDigit()).foregroundStyle(.tertiary).fixedSize()
            }
        }
        .lineLimit(1)
        .padding(.horizontal, 16)
        .frame(height: Self.height)
        .background(Color(nsColor: .windowBackgroundColor))
        .overlay(alignment: .top) { Rectangle().fill(.separator).frame(height: 1) }
    }
}
