import Foundation
import PostioFFI
import SwiftUI

/// The search dropdown's content (specs/010-focus-search step 2; design
/// §2, screens 01 and 03): sections with a bold secondary title and a
/// tertiary note, rows 38 tall inset 6 from the panel's edge, pills, the
/// cheat sheet's grid and example, and a footer of keys with the count.
///
/// It draws `DropdownModel` and decides nothing: every word is the
/// controller's, every keycap the keymap's. Heights are arithmetic
/// (`DropdownView.height`), as the bar's lines are, so the panel's size
/// needs no measuring.
///
/// Matched words get the find highlight, never the accent (FR-005); the
/// accent is the focused row's ring and fill.
public struct DropdownView: View {
    let model: DropdownModel
    let run: (UInt64) -> Void

    @Environment(\.colorScheme) private var scheme

    public init(model: DropdownModel, run: @escaping (UInt64) -> Void) {
        self.model = model
        self.run = run
    }

    /// The design's sizes (§2 "Panel structure").
    public enum Metrics {
        public static let header: CGFloat = 30
        public static let row: CGFloat = 38
        /// A person's row: the avatar, the name over the address line.
        public static let personRow: CGFloat = 48
        /// The "Understood as" bar (screen 05).
        public static let understood: CGFloat = 52
        public static let inset: CGFloat = 6
        public static let rowRadius: CGFloat = 6
        public static let pills: CGFloat = 36
        /// Narrow to: its title and its pills on one line.
        public static let narrow: CGFloat = 48
        public static let sheetRow: CGFloat = 26
        public static let example: CGFloat = 46
        public static let footer: CGFloat = 34
        public static let icon: CGFloat = 24
        public static let folder: CGFloat = 100
        public static let right: CGFloat = 110
        public static let gap: CGFloat = 6
        /// Columns of the cheat sheet.
        public static let sheetColumns = 4
    }

    /// How tall the dropdown is for what `model` holds.
    public static func height(for model: DropdownModel) -> CGFloat {
        var height = Metrics.footer + 1
        if !model.understood.isEmpty { height += Metrics.understood }
        for section in model.sections {
            height += sectionHeight(section)
        }
        return height + Metrics.gap
    }

    static func sectionHeight(_ section: DropdownModel.Section) -> CGFloat {
        let sheet = section.rows.filter { $0.kind == .cheatSheet }
        let example = section.rows.contains { $0.kind == .example }
        let rows = section.rows.filter { $0.kind != .cheatSheet && $0.kind != .example }
        var height: CGFloat = 0
        if isNarrow(section) {
            height += Metrics.narrow
        } else {
            if !section.title.isEmpty { height += Metrics.header }
            if !section.pills.isEmpty { height += Metrics.pills }
        }
        height += rows.reduce(0) { $0 + rowHeight($1) }
        let sheetRows = (sheet.count + Metrics.sheetColumns - 1) / Metrics.sheetColumns
        height += CGFloat(sheetRows) * Metrics.sheetRow
        if example { height += Metrics.example }
        if section.title.isEmpty, !rows.isEmpty { height += 2 * Metrics.gap }
        return height
    }

    static func rowHeight(_ row: DropdownModel.Row) -> CGFloat {
        row.kind == .person ? Metrics.personRow : Metrics.row
    }

    /// Narrow to draws its title and pills on one line.
    static func isNarrow(_ section: DropdownModel.Section) -> Bool {
        !section.pills.isEmpty && section.pills.contains { $0.op != nil }
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !model.understood.isEmpty { understoodBar }
            ScrollView(.vertical) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(model.sections) { section in
                        sectionView(section)
                    }
                }
                .padding(.bottom, Metrics.gap)
            }
            .scrollIndicators(.automatic)
            Divider()
            footer
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Search")
    }

    // MARK: sections

    @ViewBuilder
    private func sectionView(_ section: DropdownModel.Section) -> some View {
        if Self.isNarrow(section) {
            HStack(spacing: 8) {
                Text(section.title)
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(.secondary)
                    .fixedSize()
                ForEach(section.pills) { pill in pillView(pill) }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 16)
            .frame(height: Metrics.narrow)
            .overlay(alignment: .top) { Divider() }
            .overlay(alignment: .bottom) { Divider() }
        } else {
            VStack(alignment: .leading, spacing: 0) {
                if !section.title.isEmpty {
                    header(section)
                }
                if !section.pills.isEmpty {
                    HStack(spacing: 8) {
                        ForEach(section.pills) { pill in pillView(pill) }
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 16)
                    .frame(height: Metrics.pills)
                }
                let sheet = section.rows.filter { $0.kind == .cheatSheet }
                if !sheet.isEmpty { cheatSheet(sheet) }
                ForEach(section.rows.filter { $0.kind != .cheatSheet && $0.kind != .example }) { row in
                    rowView(row)
                }
                if let example = section.rows.first(where: { $0.kind == .example }) {
                    exampleView(example)
                }
            }
            .padding(.vertical, section.title.isEmpty ? Metrics.gap : 0)
        }
    }

    private func header(_ section: DropdownModel.Section) -> some View {
        HStack(spacing: 4) {
            Text(section.title)
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(.secondary)
            Spacer(minLength: 8)
            if let cap = section.noteCap {
                Text(cap).font(.system(size: 12)).foregroundStyle(.tertiary)
            }
            if let note = section.note {
                Text(note).font(.system(size: 12)).foregroundStyle(.tertiary)
            }
        }
        .lineLimit(1)
        .padding(.horizontal, 16)
        .frame(height: Metrics.header, alignment: .bottom)
        .padding(.bottom, 0)
        .accessibilityAddTraits(.isHeader)
    }

    // MARK: plain English

    /// "Understood as", one tile a term: the term in SF Mono, and under it
    /// at 10.5 what it came from (§1, screen 05).
    private var understoodBar: some View {
        HStack(spacing: 8) {
            Text(Self.words.understoodAs)
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(.secondary)
                .fixedSize()
                .padding(.trailing, 4)
            ForEach(Array(model.understood.enumerated()), id: \.offset) { _, tile in
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 0) {
                        Text(tile.op).foregroundStyle(.tertiary)
                        Text(tile.value).foregroundStyle(.primary).fontWeight(.medium)
                    }
                    .font(.system(size: 12, design: .monospaced))
                    Text(tile.origin)
                        .font(.system(size: 10.5))
                        .foregroundStyle(.secondary)
                }
                .lineLimit(1)
                .fixedSize()
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .textBackgroundColor)))
                .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator, lineWidth: 1))
            }
            Spacer(minLength: 8)
            Text(Self.words.understoodNote)
                .font(.system(size: 12))
                .foregroundStyle(.tertiary)
                .lineLimit(1)
        }
        .padding(.horizontal, 16)
        .frame(height: Metrics.understood)
        .background(Color(nsColor: .windowBackgroundColor))
        .overlay(alignment: .bottom) { Divider() }
        .accessibilityElement(children: .combine)
    }

    // MARK: rows

    @ViewBuilder
    private func leading(_ row: DropdownModel.Row) -> some View {
        switch row.kind {
        case .person:
            Text(row.initials ?? "?")
                .font(.system(size: 10.5, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: Metrics.icon, height: Metrics.icon)
                .background(Circle().fill(Self.avatar(row.title.map(\.text).joined())))
        case .word:
            Text("Aa")
                .font(.system(size: 10.5, weight: .bold))
                .foregroundStyle(.secondary)
                .frame(width: Metrics.icon, height: Metrics.icon)
                .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary))
        case .label, .list, .file, .folder:
            Image(systemName: Self.symbol(for: row.kind))
                .font(.system(size: row.kind == .label ? 8 : 11))
                .foregroundStyle(row.kind == .label ? AnyShapeStyle(Self.labelDot) : AnyShapeStyle(.secondary))
                .frame(width: Metrics.icon, height: Metrics.icon)
                .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary))
        default:
            Image(systemName: Self.symbol(for: row.kind))
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .frame(width: Metrics.icon)
        }
    }

    /// The chrome's words, composed in Rust.
    static let words = focusSearchWords()

    /// The design's label dot (screen 02): the system's orange, as a
    /// label's own colour is not read here.
    static let labelDot = Color.orange

    /// A person's avatar colour: the popovers' (`PopoverRowView.avatar`),
    /// so one person is one colour across search.
    static func avatar(_ name: String) -> Color {
        PopoverRowView.avatar(name)
    }

    private func rowView(_ row: DropdownModel.Row) -> some View {
        let focused = model.highlighted == row.id
        return HStack(spacing: 0) {
            leading(row)
                .padding(.trailing, row.kind == .person || row.kind == .word ? 10 : 0)
            Group {
                if row.kind == .person {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(attributed(row.title, size: 13.5)).lineLimit(1)
                        Text(attributed(row.detail, size: 11.5, secondary: true))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                } else {
                    HStack(spacing: 8) {
                        Text(attributed(row.title, size: 14))
                            .lineLimit(1)
                            .layoutPriority(1)
                        if !row.detail.isEmpty {
                            Text(attributed(row.detail, size: 12.5, secondary: true))
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if row.kind != .showAll {
                Text(row.folder ?? "")
                    .font(.system(size: 11.5, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(width: Metrics.folder, alignment: .leading)
            }
            HStack(spacing: 6) {
                Spacer(minLength: 0)
                if let right = row.right {
                    Text(right).font(.system(size: 12.5).monospacedDigit()).foregroundStyle(.secondary)
                }
                if let cap = row.cap { KeyCap(cap) }
            }
            .lineLimit(1)
            .frame(width: Metrics.right, alignment: .trailing)
        }
        .padding(.leading, 6)
        .padding(.trailing, 10)
        .frame(height: Self.rowHeight(row))
        .background(
            RoundedRectangle(cornerRadius: Metrics.rowRadius)
                .fill(focused ? AnyShapeStyle(.tint.opacity(0.09)) : AnyShapeStyle(.clear))
        )
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.rowRadius)
                .strokeBorder(.tint, lineWidth: 2)
                .opacity(focused ? 1 : 0)
        )
        .padding(.horizontal, Metrics.inset)
        .contentShape(Rectangle())
        .onTapGesture { if row.selectable { run(row.id) } }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(focused ? [.isSelected, .isButton] : .isButton)
    }

    static func symbol(for kind: DropdownRowKindFfi) -> String {
        switch kind {
        case .recent: return "arrow.counterclockwise"
        case .hit: return "envelope"
        case .showAll: return "line.3.horizontal"
        case .cheatSheet, .example: return "text.magnifyingglass"
        case .word: return "textformat"
        case .label: return "circle.fill"
        case .list: return "list.bullet"
        case .file: return "doc"
        case .person: return "person.crop.circle"
        case .folder: return "folder"
        }
    }

    // MARK: pills

    private func pillView(_ pill: DropdownModel.Pill) -> some View {
        Button { run(pill.id) } label: {
            HStack(spacing: 5) {
                if let op = pill.op {
                    Text(op).font(.system(size: 12, design: .monospaced)).foregroundStyle(.tertiary)
                    Text(pill.label).font(.system(size: 12.5)).foregroundStyle(.primary)
                } else {
                    Text(pill.label).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(.primary)
                }
                if let count = pill.count {
                    Text(count).font(.system(size: 12).monospacedDigit()).foregroundStyle(.secondary)
                }
                // A quiet badge, never a banner (D15): the accent's tint,
                // small, after the count.
                if let fresh = pill.fresh {
                    Text(fresh)
                        .font(.system(size: 10.5, weight: .semibold).monospacedDigit())
                        .foregroundStyle(.tint)
                        .padding(.horizontal, 5)
                        .frame(height: 16)
                        .background(Capsule().fill(.tint.opacity(0.12)))
                }
                if let cap = pill.cap { KeyCap(cap) }
            }
            .lineLimit(1)
            .padding(.horizontal, 10)
            .frame(height: 26)
            // Narrow to's pills are filled capsules; a saved search's is an
            // outlined key-like box (screens 03 and 01).
            .background(
                RoundedRectangle(cornerRadius: pill.op != nil ? 13 : 6)
                    .fill(pill.op != nil ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear))
            )
            .overlay(
                RoundedRectangle(cornerRadius: pill.op != nil ? 13 : 6)
                    .strokeBorder(.separator, lineWidth: pill.op != nil ? 0 : 1)
            )
        }
        .buttonStyle(.plain)
        .fixedSize()
    }

    // MARK: the cheat sheet

    private func cheatSheet(_ rows: [DropdownModel.Row]) -> some View {
        let columns = Array(
            repeating: GridItem(.flexible(), spacing: 12, alignment: .leading),
            count: Metrics.sheetColumns)
        return LazyVGrid(columns: columns, alignment: .leading, spacing: 0) {
            ForEach(rows) { row in
                HStack(spacing: 6) {
                    Text(attributed(row.title, size: 12.5))
                    Text(attributed(row.detail, size: 12.5, secondary: true))
                }
                .lineLimit(1)
                .frame(height: Metrics.sheetRow, alignment: .leading)
            }
        }
        .padding(.horizontal, 16)
    }

    private func exampleView(_ row: DropdownModel.Row) -> some View {
        HStack(spacing: 4) {
            Text(attributed(row.title, size: 12.5, secondary: true))
            Text(attributed(row.detail, size: 12.5, secondary: true))
            Spacer(minLength: 0)
        }
        .lineLimit(1)
        .padding(.horizontal, 12)
        .frame(height: 36)
        .background(RoundedRectangle(cornerRadius: 6).fill(.quinary))
        .padding(.horizontal, 16)
        .frame(height: Metrics.example)
    }

    // MARK: the footer

    private var footer: some View {
        HStack(spacing: 12) {
            ForEach(Array(model.hints.enumerated()), id: \.offset) { _, hint in
                HStack(spacing: 4) {
                    KeyCap(hint.cap)
                    Text(hint.words).font(.system(size: 12)).foregroundStyle(.secondary)
                }
                .lineLimit(1)
            }
            Spacer(minLength: 8)
            if let count = model.count {
                Text(count).font(.system(size: 12).monospacedDigit()).foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 16)
        .frame(height: Metrics.footer)
    }

    // MARK: runs

    /// The find highlight (FR-005): the system yellow, which is the
    /// design's `rgb(255,204,0)` in light and `rgb(255,214,10)` in dark, at
    /// 38% and 28%.
    private var findHighlight: Color {
        Color.yellow.opacity(scheme == .dark ? 0.28 : 0.38)
    }

    private func attributed(_ runs: [RunFfi], size: CGFloat, secondary: Bool = false) -> AttributedString {
        var text = AttributedString()
        for run in runs {
            var piece = AttributedString(run.text)
            switch run.style {
            case .strong:
                piece.font = .system(size: size, weight: .semibold)
                piece.foregroundColor = .primary
            case .mono:
                piece.font = .system(size: size - 0.5, design: .monospaced)
                piece.foregroundColor = .primary
            case .plain:
                piece.font = .system(size: size)
                piece.foregroundColor = secondary ? .secondary : .primary
            }
            if run.highlighted {
                piece.backgroundColor = findHighlight
                piece.foregroundColor = .primary
            }
            text += piece
        }
        return text
    }
}
