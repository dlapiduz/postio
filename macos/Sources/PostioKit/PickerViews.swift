import Foundation
import PostioFFI
import SwiftUI

/// A picker at the row (specs/009-focus-macos T092; screens 11 to 14): the
/// title and what it acts on, the filter above the rows or the date field
/// under them, the rows by section -- each with its dot, its detail and
/// its number key -- and the footnote saying what happens next.
///
/// The field is AppKit's (`PickerPopover` owns it and hands it in as
/// `field`), because it has to take and give up the keyboard when the
/// controller says (`FocusPickerField`) and hand ↑/↓ and Return to
/// `PickerModel`; this draws the model around it and runs a click. Rows
/// have fixed heights, so the list's height is arithmetic and scrolls past
/// a cap.
public struct PickerView<Field: View>: View {
    let model: PickerModel
    let field: (PickerFieldFfi) -> Field

    /// `field` is the AppKit field for the kind asked for: the filter, or
    /// the date field.
    public init(model: PickerModel, @ViewBuilder field: @escaping (PickerFieldFfi) -> Field) {
        self.model = model
        self.field = field
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            if model.field == .filter {
                field(.filter)
                    .frame(height: PickerMetrics.field)
                    .padding(.horizontal, 8)
                    .padding(.bottom, 6)
            }
            rows
            if model.field == .date {
                dateField
            }
            Divider()
            Text(model.footnote)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(width: PickerMetrics.width)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(model.title)
    }

    /// "Snooze until" on the left, what it acts on on the right; either
    /// wraps rather than being cut.
    private var header: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(model.title)
                .font(.system(size: 13.5, weight: .bold))
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 0)
            Text(model.target)
                .font(.system(size: 12.5))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.trailing)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .padding(.bottom, 8)
    }

    @ViewBuilder
    private var rows: some View {
        if model.rows.isEmpty {
            if !model.typed.isEmpty {
                Text(nothing)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .frame(maxWidth: .infinity, minHeight: PickerMetrics.row, alignment: .leading)
            }
        } else {
            ScrollViewReader { proxy in
                ScrollView(.vertical) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(model.rows) { row in
                            if let section = row.section {
                                Text(section)
                                    .font(.system(size: 12, weight: .semibold))
                                    .foregroundStyle(.secondary)
                                    .padding(.horizontal, 16)
                                    .frame(height: PickerMetrics.section, alignment: .bottom)
                                    .accessibilityAddTraits(.isHeader)
                            }
                            PickerRowView(row: row, highlighted: model.highlighted == row.id)
                                .id(row.id)
                                .onTapGesture { model.choose(row.id) }
                        }
                    }
                    .padding(.vertical, 4)
                }
                .scrollIndicators(.automatic)
                .frame(height: min(PickerMetrics.rowsHeight(for: model), PickerMetrics.rowsCap))
                .onChange(of: model.highlighted) { _, token in
                    guard let token else { return }
                    proxy.scrollTo(token)
                }
            }
        }
    }

    /// What an empty list says under words typed into the filter.
    private var nothing: String {
        let thing = model.kind == .move ? "folder" : "label"
        return "No \(thing) matches \u{201c}\(model.typed)\u{201d}"
    }

    /// The typed date: the field, and under it the controller's line --
    /// "Tab to type" until it has the keyboard, then the date the words
    /// make, or what it wants.
    private var dateField: some View {
        VStack(alignment: .leading, spacing: 2) {
            field(.date).frame(height: 22)
            if let hint = model.hint {
                Text(hint)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .accessibilityLabel(hint)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.6)))
        .overlay(
            RoundedRectangle(cornerRadius: 8).strokeBorder(.tint, lineWidth: 1.5)
                .opacity(model.inField ? 1 : 0)
        )
        .padding(.horizontal, 8)
        .padding(.top, 2)
        .padding(.bottom, 8)
    }
}

/// The picker's fixed metrics: the width and the rows' heights, so the
/// popover's size is arithmetic where it can be.
public enum PickerMetrics {
    /// The pack's 380.
    public static let width: CGFloat = 380
    public static let row: CGFloat = 34
    public static let section: CGFloat = 24
    /// The most the rows take before they scroll: a move picker lists
    /// every folder.
    public static let rowsCap: CGFloat = 340
    /// The field's own height, in its band.
    public static let field: CGFloat = 28
    /// The leading room before a row's name, where a label's dot goes.
    public static let dotColumn: CGFloat = 26

    /// The rows' height for what `model` holds, before the cap.
    @MainActor
    public static func rowsHeight(for model: PickerModel) -> CGFloat {
        if model.rows.isEmpty { return model.typed.isEmpty ? 0 : row }
        return model.rows.reduce(CGFloat(8)) { height, line in
            height + row + (line.section == nil ? 0 : section)
        }
    }
}

/// One row: a label's dot, the name in bold, the detail on the right and
/// the number key's cap.
private struct PickerRowView: View {
    let row: PickerModel.Row
    let highlighted: Bool

    var body: some View {
        HStack(spacing: 0) {
            ZStack {
                if row.dot {
                    Circle()
                        .fill(row.colour.map { AnyShapeStyle($0.swatch) } ?? AnyShapeStyle(.secondary))
                        .frame(width: 8, height: 8)
                } else if row.create {
                    Image(systemName: "plus").font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(.secondary)
                }
            }
            .frame(width: PickerMetrics.dotColumn, alignment: .center)
            Text(row.name)
                .font(.system(size: 14, weight: row.create ? .regular : .bold))
                .foregroundStyle(.primary)
            Spacer(minLength: 12)
            Text(row.detail)
                .font(.system(size: 13).monospacedDigit())
                .foregroundStyle(.secondary)
            Text(row.cap ?? "")
                .font(.system(size: 10, design: .monospaced))
                .foregroundStyle(.tertiary)
                .frame(width: 26, alignment: .trailing)
        }
        .lineLimit(1)
        .padding(.leading, 6)
        .padding(.trailing, 8)
        .frame(height: PickerMetrics.row)
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
        .accessibilityLabel(accessibility)
    }

    private var accessibility: String {
        var said = row.name
        if !row.detail.isEmpty { said += ", \(row.detail)" }
        if let cap = row.cap { said += ", key \(cap)" }
        return said
    }
}
