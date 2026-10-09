import PostioFFI
import SwiftUI

/// The results' filter bar (specs/010-focus-search T068; design §3.2,
/// screens 06 and 07): 46 tall on the window background with a hairline
/// below -- the tabs as a segmented control with their counts, a divider,
/// the eight filter buttons, and Sort on the right.
///
/// The buttons are the query's (`SearchQueryModel`): an applied one is
/// solid -- label colour fill, inverted words, its value as its label --
/// and the one whose popover is open carries the accent ring. The tabs
/// and the order are the results' frame (`FocusResults`). A click is
/// handed down; nothing here changes until the controller answers.
public struct FilterBarView: View {
    let query: SearchQueryModel
    let tabs: [TabFfi]
    let order: ConversationOrderFfi
    let words: SearchWordsFfi
    let pickTab: (ResultsTabFfi) -> Void
    let pickOrder: (ConversationOrderFfi) -> Void

    public init(
        query: SearchQueryModel, tabs: [TabFfi], order: ConversationOrderFfi, words: SearchWordsFfi,
        pickTab: @escaping (ResultsTabFfi) -> Void, pickOrder: @escaping (ConversationOrderFfi) -> Void
    ) {
        self.query = query
        self.tabs = tabs
        self.order = order
        self.words = words
        self.pickTab = pickTab
        self.pickOrder = pickOrder
    }

    /// The bar's height (§3.2).
    public static let height: CGFloat = 46

    public var body: some View {
        HStack(spacing: 6) {
            tabStrip
            Rectangle().fill(.separator).frame(width: 1, height: 18).padding(.horizontal, 6)
            ForEach(query.buttons) { button in
                FilterButton(button: button) { query.tap(button.kind) }
            }
            Spacer(minLength: 8)
            Text(words.sort).font(.system(size: 12)).foregroundStyle(.tertiary)
            sortMenu
        }
        .lineLimit(1)
        .padding(.leading, 16)
        .padding(.trailing, 14)
        .frame(height: Self.height)
        .background(Color(nsColor: .windowBackgroundColor))
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
    }

    // MARK: tabs

    private var tabStrip: some View {
        HStack(spacing: 2) {
            ForEach(tabs, id: \.tab) { tab in
                Button { pickTab(tab.tab) } label: {
                    HStack(spacing: 6) {
                        Text(tab.label).fontWeight(tab.selected ? .bold : .regular)
                        Text(tab.count).foregroundStyle(.tertiary)
                    }
                    .font(.system(size: 12.5))
                    .padding(.horizontal, 10)
                    .frame(height: 26)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(tab.selected ? AnyShapeStyle(Color(nsColor: .textBackgroundColor)) : AnyShapeStyle(.clear))
                            .shadow(color: .black.opacity(tab.selected ? 0.12 : 0), radius: 1, y: 0.5)
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(tab.selected ? [.isSelected, .isButton] : .isButton)
                .accessibilityLabel("\(tab.label), \(tab.count)")
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
    }

    // MARK: sort

    private var sortMenu: some View {
        Menu {
            Button(words.bestMatch) { pickOrder(.bestMatch) }
            Button(words.newest) { pickOrder(.newest) }
        } label: {
            Text(order == .bestMatch ? words.bestMatch : words.newest)
                .font(.system(size: 12.5, weight: .semibold))
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .accessibilityLabel("\(words.sort): \(order == .bestMatch ? words.bestMatch : words.newest)")
    }
}

/// One filter button: a 28-tall pill with a 1 pt outline, solid once
/// applied, ringed in the accent while its popover is open, a ▾ when it
/// opens one.
struct FilterButton: View {
    let button: SearchQueryModel.Button
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Text(button.label)
                    .font(.system(size: 12.5, weight: button.applied ? .semibold : .regular))
                if !button.isToggle {
                    Image(systemName: "chevron.down").font(.system(size: 8.5, weight: .semibold))
                }
            }
            .foregroundStyle(button.applied ? AnyShapeStyle(Color(nsColor: .textBackgroundColor)) : AnyShapeStyle(.primary))
            .padding(.leading, 10)
            .padding(.trailing, button.isToggle ? 10 : 8)
            .frame(height: 28)
            .background(Capsule().fill(button.applied ? AnyShapeStyle(.primary) : AnyShapeStyle(.clear)))
            .overlay(
                Capsule().strokeBorder(
                    button.open ? AnyShapeStyle(.tint) : AnyShapeStyle(.separator),
                    lineWidth: button.open ? 2 : 1)
                    .opacity(button.applied && !button.open ? 0 : 1)
            )
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .fixedSize()
        .accessibilityAddTraits(button.applied ? [.isSelected, .isButton] : .isButton)
    }
}
