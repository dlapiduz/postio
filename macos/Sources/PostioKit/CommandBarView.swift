import Foundation
import PostioFFI
import SwiftUI

/// The command bar's content (specs/009-focus-macos T085; screens 07 to
/// 09): the saved searches, the chips and what was typed, the lines with
/// their keycaps, and a footer naming the keys.
///
/// It draws `CommandBarModel` and decides nothing. The words are the
/// controller's; the field the person types in is the toolbar's, which
/// keeps the keyboard, so this draws the chips the words were lowered to
/// rather than a second field.
///
/// Every row has a fixed height by its kind, so the panel's height is
/// arithmetic (`CommandBarView.height`) rather than a measurement: the
/// same rule as the list's rows.
///
/// Semantic colours only. The accent is the highlight's ring and the chip
/// being edited, which are both focus.
public struct CommandBarView: View {
    let model: CommandBarModel
    let saveCap: String?

    /// `saveCap` is the keycap of `save_search` in force (`⌘S`).
    public init(model: CommandBarModel, saveCap: String?) {
        self.model = model
        self.saveCap = saveCap
    }

    /// Fixed heights, by what is drawn.
    public enum Metrics {
        public static let saved: CGFloat = 44
        public static let query: CGFloat = 44
        public static let echo: CGFloat = 24
        public static let heading: CGFloat = 30
        public static let line: CGFloat = 36
        public static let footer: CGFloat = 32
        public static let radius: CGFloat = 10
        /// Where the sender and the time columns of a message line end.
        public static let sender: CGFloat = 160
        public static let time: CGFloat = 56
    }

    /// How tall the bar is for what `model` holds, before the window caps
    /// it (the lines then scroll).
    public static func height(for model: CommandBarModel) -> CGFloat {
        var height = Metrics.footer
        if !model.saved.isEmpty { height += Metrics.saved + 1 }
        if !model.chips.isEmpty {
            height += Metrics.query
            if model.echo != nil { height += Metrics.echo }
            height += 1
        }
        if model.heading != nil { height += Metrics.heading }
        for row in model.rows {
            height += row.kind == .heading ? Metrics.heading : Metrics.line
        }
        // The divider above the footer, and a little air under the last line.
        return height + 1 + 4
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !model.saved.isEmpty {
                savedRow
                Divider()
            }
            if !model.chips.isEmpty {
                queryRow
                Divider()
            }
            ScrollViewReader { proxy in
                ScrollView(.vertical) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        if let heading = model.heading {
                            HeadingRow(text: heading, emphasised: true)
                        }
                        ForEach(model.rows) { row in
                            line(row).id(row.id)
                        }
                    }
                    .padding(.bottom, 4)
                }
                .scrollIndicators(.automatic)
                .onChange(of: model.highlighted) { _, token in
                    guard let token else { return }
                    proxy.scrollTo(token)
                }
            }
            Divider()
            footer
        }
        .background(.background, in: .rect(cornerRadius: Metrics.radius))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.radius).strokeBorder(.separator, lineWidth: 1)
        )
        .clipShape(.rect(cornerRadius: Metrics.radius))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Command bar")
    }

    // MARK: the saved row

    private var savedRow: some View {
        HStack(spacing: 8) {
            Text("Saved").font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary)
            ForEach(Array(model.saved.enumerated()), id: \.offset) { index, saved in
                Button { model.runSaved(index) } label: {
                    HStack(spacing: 5) {
                        Text(saved.name).font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(.primary)
                        if let cap = saved.cap { KeyCap(cap) }
                    }
                    .padding(.horizontal, 9)
                    .frame(height: 26)
                    .overlay(Capsule().strokeBorder(.separator, lineWidth: 1))
                }
                .buttonStyle(.plain)
                // A name is never cut: the hint on the right gives way.
                .fixedSize()
            }
            Spacer(minLength: 8)
            if let saveCap {
                // Said whole or not at all: the footer names the key too.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 4) {
                        KeyCap(saveCap)
                        Text("saves the current query")
                    }
                    .fixedSize()
                    Color.clear.frame(width: 0, height: 0)
                }
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
                .layoutPriority(-1)
            }
        }
        .padding(.horizontal, 14)
        .frame(height: Metrics.saved)
    }

    // MARK: the chips

    private var queryRow: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").font(.system(size: 13))
                    .foregroundStyle(.secondary)
                ForEach(Array(model.chips.enumerated()), id: \.offset) { index, chip in
                    ChipView(text: chip, editing: model.editing == index)
                }
                Spacer(minLength: 8)
                KeyCap("Esc")
            }
            .padding(.horizontal, 14)
            .frame(height: Metrics.query)
            if let echo = model.echo {
                Text(echo)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.horizontal, 14)
                    .frame(height: Metrics.echo, alignment: .top)
            }
        }
    }

    // MARK: the lines

    @ViewBuilder
    private func line(_ row: CommandBarModel.Row) -> some View {
        switch row.kind {
        case .heading:
            HeadingRow(text: row.title, emphasised: false)
        case .message:
            LineButton(row: row, highlighted: model.highlighted == row.id, run: model.run) {
                MessageLine(row: row)
            }
        default:
            LineButton(row: row, highlighted: model.highlighted == row.id, run: model.run) {
                PlainLine(row: row)
            }
        }
    }

    // MARK: the footer

    private var footer: some View {
        HStack(spacing: 12) {
            FooterHint(cap: "↑↓", words: "move")
            FooterHint(cap: "↩", words: model.mode == .commands ? "run" : "open")
            if model.mode == .commands {
                FooterHint(cap: ">", words: "commands only")
            } else if !model.saved.isEmpty {
                FooterHint(cap: "⌥1–4", words: "saved searches")
                if let saveCap { FooterHint(cap: saveCap, words: "save query") }
            } else {
                FooterHint(cap: ">", words: "commands only")
            }
            Spacer(minLength: 8)
            Text("Local index").font(.system(size: 11)).foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 14)
        .frame(height: Metrics.footer)
    }
}

/// A section heading, or the folder's heading over its conversations:
/// the name on the left, the rest of it (after the first " · ") on the
/// right, dimmer.
private struct HeadingRow: View {
    let text: String
    let emphasised: Bool

    var body: some View {
        let parts = text.components(separatedBy: " \u{b7} ")
        HStack {
            Text(parts.first ?? text)
                .font(.system(size: emphasised ? 13 : 12, weight: .semibold))
                .foregroundStyle(emphasised ? AnyShapeStyle(.primary) : AnyShapeStyle(.secondary))
            Spacer(minLength: 8)
            if parts.count > 1 {
                Text(parts.dropFirst().joined(separator: " \u{b7} "))
                    .font(.system(size: 11).monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
        }
        .lineLimit(1)
        .padding(.horizontal, 14)
        .frame(height: CommandBarView.Metrics.heading, alignment: .bottom)
        .padding(.bottom, 0)
        .accessibilityAddTraits(.isHeader)
    }
}

/// A line that runs: the highlight's ring when it is highlighted, and a
/// click runs it.
private struct LineButton<Content: View>: View {
    let row: CommandBarModel.Row
    let highlighted: Bool
    let run: (UInt64) -> Void
    @ViewBuilder let content: () -> Content

    var body: some View {
        content()
            .padding(.horizontal, 14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .frame(height: CommandBarView.Metrics.line)
            .background(highlighted ? AnyShapeStyle(.tint.opacity(0.10)) : AnyShapeStyle(.clear))
            .overlay(
                Rectangle().strokeBorder(.tint, lineWidth: 1.5).opacity(highlighted ? 1 : 0)
            )
            .contentShape(Rectangle())
            .onTapGesture { if row.selectable { run(row.id) } }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(highlighted ? [.isSelected, .isButton] : .isButton)
    }
}

/// A command, a place, a search, a hint: the title, its detail, the key.
private struct PlainLine: View {
    let row: CommandBarModel.Row

    var body: some View {
        HStack(spacing: 8) {
            Text(row.title)
                .font(.system(size: 13))
                .foregroundStyle(row.kind == .hint ? AnyShapeStyle(.secondary) : AnyShapeStyle(.primary))
                .layoutPriority(1)
            if let detail = row.detail {
                Text(detail).font(.system(size: 12)).foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            if let cap = row.cap { KeyCap(cap) }
        }
        .lineLimit(1)
    }
}

/// A message: who from, the subject and its first line, where it is, when.
private struct MessageLine: View {
    let row: CommandBarModel.Row

    var body: some View {
        HStack(spacing: 10) {
            Text(row.sender ?? "")
                .font(.system(size: 13))
                .frame(width: CommandBarView.Metrics.sender, alignment: .leading)
            Text(row.title).font(.system(size: 13, weight: .medium)).layoutPriority(1)
            if let detail = row.detail {
                Text(detail).font(.system(size: 12)).foregroundStyle(.secondary)
                    .layoutPriority(-1)
            }
            Spacer(minLength: 8)
            // Where it is, whole: the subject's first line gives way first.
            VStack(alignment: .trailing, spacing: 0) {
                ForEach(row.wheres, id: \.self) { place in
                    Text(place).font(.system(size: row.wheres.count > 1 ? 10 : 11, design: .monospaced))
                }
            }
            .foregroundStyle(.secondary)
            .fixedSize()
            Text(row.time ?? "")
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(.secondary)
                .frame(width: CommandBarView.Metrics.time, alignment: .trailing)
                .fixedSize()
        }
        .lineLimit(1)
    }
}

/// One chip: an operator and its value, in mono; the one being edited is
/// ringed in the accent.
private struct ChipView: View {
    let text: String
    let editing: Bool

    var body: some View {
        Text(text)
            .font(.system(size: 13, design: .monospaced))
            .foregroundStyle(.primary)
            .padding(.horizontal, 8)
            .frame(height: 26)
            .background(RoundedRectangle(cornerRadius: 5).fill(.quaternary))
            .overlay(
                RoundedRectangle(cornerRadius: 5)
                    .strokeBorder(editing ? AnyShapeStyle(.tint) : AnyShapeStyle(.separator),
                                  lineWidth: editing ? 1.5 : 1)
            )
    }
}

/// A footer hint: a keycap and what it does.
private struct FooterHint: View {
    let cap: String
    let words: String

    var body: some View {
        HStack(spacing: 4) {
            KeyCap(cap)
            Text(words).font(.system(size: 11)).foregroundStyle(.secondary)
        }
        .lineLimit(1)
    }
}
