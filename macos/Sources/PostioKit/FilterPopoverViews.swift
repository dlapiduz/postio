import PostioFFI
import SwiftUI

/// What a filter popover shows (specs/010-focus-search T083; design §3.6,
/// screens 08 and 09): the list pattern for From, To, Anywhere and Label,
/// and the two columns of Date. Every row, count, word and check is the
/// controller's last `FocusPopover` (`FilterPopoverModel.view`); a click,
/// a key or a word typed goes down, and nothing here changes until it
/// answers -- but the fields' own text and the arrows' highlight.
public struct FilterPopoverContent: View {
    let model: FilterPopoverModel

    public init(model: FilterPopoverModel) {
        self.model = model
    }

    public var body: some View {
        if let view = model.view {
            if view.kind == .date {
                DatePopoverView(model: model, view: view)
            } else {
                ListPopoverView(model: model, view: view)
            }
        }
    }
}

/// The popovers' sizes (§3.6).
public enum FilterPopoverMetrics {
    /// From, To, Anywhere and Label.
    public static let listWidth: CGFloat = 360
    /// A list popover's row.
    public static let rowHeight: CGFloat = 44
    /// Date, two columns.
    public static let dateWidth: CGFloat = 470
    /// Date's presets column.
    public static let presetsWidth: CGFloat = 170
    /// Date's month chart.
    public static let chartHeight: CGFloat = 90
}

// MARK: From, To, Anywhere, Label

/// The list pattern (screen 08): its own search field, rows 44 tall with a
/// checkbox, an avatar or an icon, the name over the address, and a bar
/// with the count; the keys in a footer. Space toggles the highlighted
/// row while the field is empty (with words in it, Space is a space);
/// ⌥-click excludes.
struct ListPopoverView: View {
    let model: FilterPopoverModel
    let view: PopoverViewFfi
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            field.padding(8)
            VStack(spacing: 0) {
                ForEach(Array(view.rows.enumerated()), id: \.element.token) { at, row in
                    PopoverRowView(row: row, kind: view.kind, highlighted: model.highlight == at)
                        .contentShape(Rectangle())
                        .highPriorityGesture(
                            TapGesture().modifiers(.option).onEnded {
                                model.toggle(token: row.token, exclude: true)
                            })
                        .onTapGesture { model.toggle(token: row.token, exclude: false) }
                }
            }
            .padding(.horizontal, 6)
            .padding(.bottom, 6)
            Rectangle().fill(.separator).frame(height: 1)
            PopoverFooter(hints: view.hints)
        }
        .frame(width: FilterPopoverMetrics.listWidth)
        .onAppear { fieldFocused = true }
    }

    private var field: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.tertiary)
            TextField(
                view.placeholder,
                text: Binding(get: { model.filterText }, set: { model.filter($0) })
            )
            .textFieldStyle(.plain)
            .font(.system(size: 13.5))
            .focused($fieldFocused)
            .onSubmit { model.apply() }
            .onExitCommand { model.cancel() }
            .onKeyPress(.downArrow) {
                model.moveHighlight(by: 1)
                return .handled
            }
            .onKeyPress(.upArrow) {
                model.moveHighlight(by: -1)
                return .handled
            }
            .onKeyPress(.space) {
                guard model.filterText.isEmpty else { return .ignored }
                model.toggleHighlighted()
                return .handled
            }
        }
        .padding(.horizontal, 10)
        .frame(height: 36)
        .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
    }
}

/// One person, folder or label in a list popover.
struct PopoverRowView: View {
    let row: PopoverRowFfi
    let kind: FilterKindFfi
    let highlighted: Bool

    var body: some View {
        HStack(spacing: 10) {
            checkbox
            leading
            VStack(alignment: .leading, spacing: 1) {
                Text(row.title)
                    .font(.system(size: 13, weight: .semibold))
                    .strikethrough(row.excluded)
                if let detail = row.detail {
                    Text(detail)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.tertiary)
                }
            }
            .lineLimit(1)
            Spacer(minLength: 8)
            Capsule()
                .fill(.tertiary)
                .frame(width: max(6, 60 * row.share), height: 3)
                .frame(width: 60, alignment: .trailing)
            Text(String(row.count))
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .monospacedDigit()
                .frame(minWidth: 22, alignment: .trailing)
        }
        .padding(.horizontal, 8)
        .frame(height: FilterPopoverMetrics.rowHeight)
        .background(
            RoundedRectangle(cornerRadius: 7)
                .fill(highlighted ? AnyShapeStyle(Color.accentColor.opacity(0.08)) : AnyShapeStyle(.clear))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 7)
                .strokeBorder(Color.accentColor, lineWidth: 2)
                .opacity(highlighted ? 1 : 0)
        )
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(row.checked ? [.isSelected, .isButton] : .isButton)
    }

    private var checkbox: some View {
        let symbol = row.checked ? "checkmark.square.fill" : (row.excluded ? "minus.square.fill" : "square")
        return Image(systemName: symbol)
            .font(.system(size: 15))
            .foregroundStyle(row.checked || row.excluded ? AnyShapeStyle(.primary) : AnyShapeStyle(.tertiary))
            .frame(width: 18)
    }

    @ViewBuilder private var leading: some View {
        switch kind {
        case .anywhere:
            Image(systemName: "folder").font(.system(size: 14)).foregroundStyle(.secondary).frame(width: 28)
        case .label:
            Circle().fill(labelColour).frame(width: 10, height: 10).frame(width: 28)
        default:
            Text(row.initials ?? "")
                .font(.system(size: 10.5, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: 28, height: 28)
                .background(Circle().fill(Self.avatar(row.title)))
        }
    }

    private var labelColour: AnyShapeStyle {
        LabelColour(hex: row.color).map { AnyShapeStyle($0.swatch) } ?? AnyShapeStyle(.secondary)
    }

    /// An avatar's fill: one of the system's hues, by the name, so a person
    /// keeps theirs. Drawing only: nothing reads it.
    static func avatar(_ name: String) -> Color {
        let hues: [Color] = [.orange, .green, .gray, .purple, .blue, .brown]
        let sum = name.unicodeScalars.reduce(0) { ($0 &* 31 &+ Int($1.value)) & 0xffff }
        return hues[sum % hues.count].opacity(0.85)
    }
}

/// A popover's footer of keys: "Space toggle  ⌥ -click excludes  ↩ apply".
struct PopoverFooter: View {
    let hints: [KeyHintFfi]

    var body: some View {
        HStack(spacing: 12) {
            ForEach(Array(hints.enumerated()), id: \.offset) { at, hint in
                if at == hints.count - 1, hints.count > 1 { Spacer(minLength: 0) }
                HStack(spacing: 4) {
                    KeyCap(KeyCapSpelling.cap(hint.key) ?? hint.key)
                    Text(hint.label).font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 32)
    }
}

// MARK: Date

/// The Date popover (screen 09): the presets with their counts on the
/// left; on the right the plain-words field and what it became, the month
/// chart to drag, and what the dates keep.
struct DatePopoverView: View {
    let model: FilterPopoverModel
    let view: PopoverViewFfi
    @FocusState private var fieldFocused: Bool
    @State private var drag: TimelineDrag

    init(model: FilterPopoverModel, view: PopoverViewFfi) {
        self.model = model
        self.view = view
        _drag = State(initialValue: TimelineDrag(bars: max(view.months.count, 12)) { [model] in
            model.months($0, $1)
        })
    }

    var body: some View {
        HStack(spacing: 0) {
            presets
                .frame(width: FilterPopoverMetrics.presetsWidth)
                .frame(maxHeight: .infinity, alignment: .top)
                .background(Color(nsColor: .windowBackgroundColor))
            Rectangle().fill(.separator).frame(width: 1)
            right.padding(14)
        }
        .frame(width: FilterPopoverMetrics.dateWidth)
        .fixedSize(horizontal: false, vertical: true)
        .onAppear { fieldFocused = true }
    }

    private var presets: some View {
        VStack(spacing: 2) {
            ForEach(view.presets, id: \.token) { preset in
                Button { model.preset(preset.token) } label: {
                    HStack {
                        Text(preset.label)
                            .font(.system(size: 13, weight: preset.selected ? .semibold : .regular))
                        Spacer()
                        if let count = preset.count {
                            Text(count).font(.system(size: 12)).foregroundStyle(.tertiary).monospacedDigit()
                        }
                    }
                    .padding(.horizontal, 10)
                    .frame(height: 30)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(preset.selected ? AnyShapeStyle(Color(nsColor: .textBackgroundColor)) : AnyShapeStyle(.clear))
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .strokeBorder(Color.accentColor, lineWidth: 2)
                            .opacity(preset.selected ? 1 : 0)
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(preset.selected ? [.isSelected, .isButton] : .isButton)
            }
        }
        .padding(8)
    }

    private var right: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                TextField("", text: Binding(get: { model.wordsText }, set: { model.words($0) }))
                    .textFieldStyle(.plain)
                    .font(.system(size: 14))
                    .focused($fieldFocused)
                    .onSubmit { model.apply() }
                    .onExitCommand { model.cancel() }
                if let parsed = view.parsed {
                    Text(parsed)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .fixedSize()
                }
            }
            .padding(.horizontal, 10)
            .frame(height: 32)
            .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .textBackgroundColor)))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator, lineWidth: 1))
            Text(view.wordsHint)
                .font(.system(size: 11.5))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            MonthChart(
                months: view.months, barHeight: FilterPopoverMetrics.chartHeight, labelSize: 10,
                drag: drag)
            HStack(spacing: 8) {
                if let result = view.result {
                    Text(result).font(.system(size: 13, weight: .bold))
                }
                if let range = view.range {
                    Text(range).font(.system(size: 12)).foregroundStyle(.secondary)
                }
                Spacer()
                PopoverFooter(hints: view.hints).padding(.horizontal, -12)
            }
            .frame(height: 22)
        }
    }
}
