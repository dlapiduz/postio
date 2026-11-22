import PostioFFI
import SwiftUI

/// Keys go through the resolver; a click tells the controller, which moves the ring
/// (`FocusResultsCursor`). Nothing moves here on its own.
public struct PeopleListView: View {
    let model: ResultsModel

    public init(model: ResultsModel) {
        self.model = model
    }

    public enum Columns {
        public static let avatar: CGFloat = 32
        public static let messages: CGFloat = 110
        public static let date: CGFloat = ResultRowView.Columns.date
        public static let gap: CGFloat = ResultRowView.Columns.gap
        public static let leading: CGFloat = ResultRowView.Columns.leading + 8
        public static let trailing: CGFloat = ResultRowView.Columns.trailing
    }

    public var body: some View {
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

public struct PersonRowView: View {
    @Environment(\.colorScheme) private var scheme
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
            // The accent's tint under the ring, never as a selection.
            if focused { Color.accentColor.opacity(SearchRuns.focusFill(scheme)) }
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
