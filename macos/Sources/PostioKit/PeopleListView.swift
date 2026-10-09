import PostioFFI
import SwiftUI

/// The People tab (specs/010-focus-search T133; design §3.11, not drawn):
/// everyone the results are from or to, you left out, most messages
/// first, as a list in the table's place.
///
/// Each row keeps a result row's rhythm -- 58 tall, the hairline under
/// it, the accent ring and tint on the focused one -- and the From
/// popover's person: the initials avatar in its colour, the name over the
/// address. Then how many matched messages are with them, and the newest
/// one's date in the date column.
///
/// Every word is the controller's (`PersonRowFfi`). Keys go through the
/// resolver -- j/k and the arrows move the ring, ↩ searches `from:` them
/// -- and a click tells the controller, which moves the ring
/// (`FocusResultsCursor`); nothing moves here on its own.
public struct PeopleListView: View {
    let model: ResultsModel

    public init(model: ResultsModel) {
        self.model = model
    }

    /// The columns: a result row's leading and trailing, its 62 date.
    public enum Columns {
        public static let avatar: CGFloat = 32
        public static let messages: CGFloat = 110
        public static let date: CGFloat = ResultRowView.Columns.date
        public static let gap: CGFloat = ResultRowView.Columns.gap
        public static let leading: CGFloat = ResultRowView.Columns.leading + 8
        public static let trailing: CGFloat = ResultRowView.Columns.trailing
    }

    public var body: some View {
        // Read so a new frame draws the rows again.
        let _ = model.frame
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(0..<model.personCount, id: \.self) { at in
                        let position = UInt64(at)
                        PersonRowView(row: model.person(at: position), focused: model.cursor == position)
                            .frame(height: ResultsModel.Metrics.row)
                            .contentShape(Rectangle())
                            .onTapGesture(count: 2) { model.open(person: position) }
                            .simultaneousGesture(TapGesture().onEnded { model.point(person: position) })
                            .id(position)
                    }
                }
            }
            .onChange(of: model.cursor) { _, cursor in
                guard let cursor else { return }
                proxy.scrollTo(cursor)
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(focusSearchWords().peopleLabel)
        .accessibilityAddTraits(.isSummaryElement)
    }
}

/// One person (design §3.11): the avatar, the name over the address, the
/// count, the last date.
public struct PersonRowView: View {
    let row: PersonRowFfi?
    let focused: Bool

    public init(row: PersonRowFfi?, focused: Bool) {
        self.row = row
        self.focused = focused
    }

    public var body: some View {
        HStack(spacing: PeopleListView.Columns.gap) {
            if let row {
                Text(row.initials)
                    .font(.system(size: 11.5, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: PeopleListView.Columns.avatar, height: PeopleListView.Columns.avatar)
                    .background(Circle().fill(PopoverRowView.avatar(row.name)))
                VStack(alignment: .leading, spacing: 3) {
                    Text(row.name)
                        .font(.system(size: 13.5, weight: .semibold))
                        .foregroundStyle(.primary)
                    Text(row.address)
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(.secondary)
                }
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
                Text(row.messages)
                    .font(.system(size: 12).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .frame(width: PeopleListView.Columns.messages, alignment: .trailing)
                Text(row.last)
                    .font(.system(size: 12).monospacedDigit())
                    .foregroundStyle(.secondary)
                    .fixedSize()
                    .frame(minWidth: PeopleListView.Columns.date, alignment: .trailing)
            } else {
                Spacer()
            }
        }
        .lineLimit(1)
        .padding(.leading, PeopleListView.Columns.leading)
        .padding(.trailing, PeopleListView.Columns.trailing)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background {
            // A result row's ground: the accent's tint under the ring,
            // never as a selection.
            if focused { Color.accentColor.opacity(0.07) }
        }
        .overlay(alignment: .bottom) {
            Rectangle().fill(.separator).frame(height: 1)
        }
        .overlay {
            Rectangle()
                .strokeBorder(Color.accentColor, lineWidth: 2)
                .opacity(focused ? 1 : 0)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(row?.accessible ?? "")
        .accessibilityAddTraits(focused ? [.isSelected, .isButton] : .isButton)
    }
}
