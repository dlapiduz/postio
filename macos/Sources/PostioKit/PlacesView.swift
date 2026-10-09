import Foundation
import PostioFFI
import SwiftUI

/// The folders and labels popover's list (specs/009-focus-macos T086,
/// screen 10): the places by section, each with its mark, its count and
/// its direct key, and the footer saying what Return does on the
/// highlighted one.
///
/// The filter field above it is AppKit's (`PlacesPopover`), which keeps the
/// keyboard and hands the arrows and Return to `PlacesModel`; this draws the
/// model and runs a click. Rows have fixed heights, so the popover's size
/// is arithmetic (`PlacesView.height`).
public struct PlacesView: View {
    let model: PlacesModel
    let open: (UInt64) -> Void

    /// `open` is a click on a row: go there, and close the popover.
    public init(model: PlacesModel, open: @escaping (UInt64) -> Void) {
        self.model = model
        self.open = open
    }

    public enum Metrics {
        public static let width: CGFloat = 392
        public static let heading: CGFloat = 26
        public static let row: CGFloat = 30
        public static let footer: CGFloat = 40
        public static let empty: CGFloat = 44
        /// The filter field's band above the list.
        public static let field: CGFloat = 52
    }

    /// The list and its footer's height for what `model` holds, before
    /// the popover caps it (the list then scrolls).
    public static func height(for model: PlacesModel) -> CGFloat {
        guard !model.entries.isEmpty else { return Metrics.empty + Metrics.footer + 1 }
        let rows = model.entries.reduce(CGFloat(0)) { height, entry in
            height + Metrics.row + (entry.heading == nil ? 0 : Metrics.heading)
        }
        return rows + 8 + Metrics.footer + 1
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if model.entries.isEmpty {
                Text("No folder or label matches \u{201c}\(model.filter)\u{201d}")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .frame(maxWidth: .infinity, minHeight: Metrics.empty, alignment: .leading)
            } else {
                ScrollViewReader { proxy in
                    ScrollView(.vertical) {
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(model.entries.enumerated()), id: \.element.id) { index, entry in
                                if let heading = entry.heading {
                                    Text(heading)
                                        .font(.system(size: 12, weight: .semibold))
                                        .foregroundStyle(.secondary)
                                        .padding(.horizontal, 16)
                                        .frame(height: Metrics.heading, alignment: .bottom)
                                        .accessibilityAddTraits(.isHeader)
                                }
                                PlaceRow(
                                    entry: entry, highlighted: model.highlighted == index,
                                    current: entry.name == model.placeName
                                )
                                .id(entry.id)
                                .onTapGesture { open(entry.id) }
                            }
                        }
                        .padding(.bottom, 8)
                    }
                    .onChange(of: model.highlighted) { _, index in
                        guard let index, model.entries.indices.contains(index) else { return }
                        proxy.scrollTo(model.entries[index].id)
                    }
                }
            }
            Divider()
            Text(model.footer)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .lineLimit(2)
                .padding(.horizontal, 16)
                .frame(maxWidth: .infinity, minHeight: Metrics.footer, alignment: .leading)
        }
        .frame(width: Metrics.width)
    }
}

/// One place: its mark, its name (bold where the list is now), its count
/// and its key.
private struct PlaceRow: View {
    let entry: PlacesModel.Entry
    let highlighted: Bool
    let current: Bool

    var body: some View {
        HStack(spacing: 10) {
            mark.frame(width: 16)
            Text(entry.name)
                .font(.system(size: 14, weight: current ? .bold : .regular))
                .foregroundStyle(.primary)
            Spacer(minLength: 8)
            if let count = entry.count {
                Text(count)
                    .font(.system(size: 12).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            Text(entry.cap ?? "")
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.tertiary)
                .frame(width: 28, alignment: .trailing)
        }
        .lineLimit(1)
        .padding(.horizontal, 10)
        .frame(height: PlacesView.Metrics.row)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(highlighted ? AnyShapeStyle(.tint.opacity(0.10)) : AnyShapeStyle(.clear))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 6).strokeBorder(.tint, lineWidth: 1.5)
                .opacity(highlighted ? 1 : 0)
        )
        .padding(.horizontal, 6)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(highlighted ? [.isSelected, .isButton] : .isButton)
    }

    @ViewBuilder
    private var mark: some View {
        switch entry.mark {
        case let .symbol(name):
            Image(systemName: name).font(.system(size: 13)).foregroundStyle(.secondary)
        case let .dot(colour):
            Circle()
                .fill(colour.map { AnyShapeStyle($0.swatch) } ?? AnyShapeStyle(.secondary))
                .frame(width: 8, height: 8)
        }
    }
}
