import SwiftUI

/// The results' footer (specs/010-focus-search T072; design §3.5): 32
/// tall with a hairline above. Normally the keys -- move, open, select,
/// edit the query, back to the inbox -- and on the right "48
/// conversations · local index · 41 ms". While results are checked it is
/// the bulk bar: "5 selected", then the verbs with their keys.
///
/// Every word and key is the controller's (`FocusResults`); the checked
/// count's words are `postio-ui`'s, handed in as `checked`.
public struct SearchFooter: View {
    let hints: [ResultsModel.Hint]
    let right: String
    /// "5 selected" while results are checked, else `nil`.
    let checked: String?
    let bulk: [ResultsModel.Hint]

    public init(hints: [ResultsModel.Hint], right: String, checked: String?, bulk: [ResultsModel.Hint]) {
        self.hints = hints
        self.right = right
        self.checked = checked
        self.bulk = bulk
    }

    public static let height: CGFloat = 32

    public var body: some View {
        HStack(spacing: 14) {
            if let checked {
                Text(checked).font(.system(size: 12, weight: .bold)).foregroundStyle(.primary)
                FocusHintRow(bulk.map { FocusHintRow.Hint(cap: $0.cap, label: $0.label) })
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
