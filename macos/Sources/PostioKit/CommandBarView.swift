import Foundation
import PostioFFI
import SwiftUI

/// Every row has a fixed height by its kind, so the panel's height is arithmetic rather than a
/// measurement.
/// Semantic colours only; the accent is the highlight's ring.
public struct CommandBarView: View {
    let model: CommandBarModel
    let saveCap: String?

    public init(model: CommandBarModel, saveCap: String?) {
        self.model = model
        self.saveCap = saveCap
    }

    public enum Metrics {
        public static let saved: CGFloat = 44
        public static let heading: CGFloat = 30
        public static let line: CGFloat = 36
        public static let footer: CGFloat = 32
        public static let radius: CGFloat = 10
    }

    public static func height(for model: CommandBarModel) -> CGFloat {
        if model.showsDropdown { return DropdownView.height(for: model.dropdown) }
        var height = Metrics.footer
        if !model.saved.isEmpty { height += Metrics.saved + 1 }
        for row in model.rows {
            height += row.kind == .heading ? Metrics.heading : Metrics.line
        }
        return height + 1 + 4
    }

    public var body: some View {
        Group {
            if model.showsDropdown {
                DropdownView(model: model.dropdown) { token in model.dropdown.run(token) }
            } else {
                lines
            }
        }
        .background(.background, in: .rect(cornerRadius: Metrics.radius))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.radius).strokeBorder(.separator, lineWidth: 1)
        )
        .clipShape(.rect(cornerRadius: Metrics.radius))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Command bar")
    }

    private var lines: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !model.saved.isEmpty {
                savedRow
                Divider()
            }
            ScrollViewReader { proxy in
                ScrollView(.vertical) {
                    LazyVStack(alignment: .leading, spacing: 0) {
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
    }

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
                .fixedSize()
            }
            Spacer(minLength: 8)
            if let saveCap {
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

    @ViewBuilder
    private func line(_ row: CommandBarModel.Row) -> some View {
        switch row.kind {
        case .heading:
            HeadingRow(text: row.title)
        case .hint, .command, .correspondent:
            LineButton(row: row, highlighted: model.highlighted == row.id, run: model.run) {
                PlainLine(row: row)
            }
        }
    }

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

private struct HeadingRow: View {
    let text: String

    var body: some View {
        let parts = text.components(separatedBy: " \u{b7} ")
        HStack {
            Text(parts.first ?? text)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
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
