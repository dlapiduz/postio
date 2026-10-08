import PostioFFI
import SwiftUI

/// Filtered, drawn in the list's place in the main window
/// (specs/009-focus-macos T113, screen 21): "‹ Inbox" and the title, the
/// reason tabs `1`-`7` with their counts and the note that nothing here is
/// deleted (C4), the rows -- sender, subject and first line, the reason
/// pill, the time -- under their day headings, and the footer's keys.
///
/// Every word is the controller's (`FocusFiltered`); the keys reach it
/// through the key monitor, and the pointer through `FilteredModel`. The
/// focused row is ringed in the accent, as the list's cursor is, and
/// carries the Restore button with its key.
public struct FilteredView: View {
    let model: FilteredModel

    public init(model: FilteredModel) {
        self.model = model
    }

    /// One row's height, as the list's one-line row: nothing is measured.
    public static let rowHeight: CGFloat = 40
    /// A day heading's height.
    public static let headingHeight: CGFloat = 32
    /// The sender column's width.
    public static let senderWidth: CGFloat = 196

    public var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            tabs
            Divider()
            rows
            Divider()
            FocusHintRow(model.footer.map { FocusHintRow.Hint(cap: $0.cap, label: $0.label) })
                .padding(.horizontal, 20)
                .frame(height: 30)
                .background(.bar)
        }
        .background(Color(nsColor: .textBackgroundColor))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(model.title)
    }

    // MARK: header

    private var header: some View {
        ZStack {
            VStack(spacing: 1) {
                Text(model.title).font(.system(size: 13.5, weight: .bold))
                Text(model.subtitle).font(.system(size: 11.5)).foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            HStack {
                Button(action: model.back) {
                    HStack(spacing: 4) {
                        Image(systemName: "chevron.left").font(.system(size: 12, weight: .semibold))
                        Text("Inbox").font(.system(size: 13.5, weight: .semibold))
                        if let cap = model.backCap { KeyCap(cap) }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Back to the inbox")
                Spacer()
                if !model.sweepWords.isEmpty {
                    FocusVerbButton(model.sweepWords, cap: model.sweepCap, bold: false, action: model.sweep)
                }
            }
            .padding(.horizontal, 16)
        }
        .frame(height: 44)
    }

    // MARK: tabs

    private var tabs: some View {
        HStack(spacing: 4) {
            ForEach(model.tabs) { tab in
                Button { model.chooseTab(tab.id) } label: {
                    HStack(spacing: 5) {
                        Text(tab.name).font(.system(size: 13, weight: tab.on ? .bold : .regular))
                        Text("\(tab.count)")
                            .font(.system(size: 12).monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 10)
                    .frame(height: 28)
                    .background {
                        if tab.on {
                            Capsule().fill(Color(nsColor: .controlBackgroundColor))
                                .overlay(Capsule().strokeBorder(.separator))
                        }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .help(tab.cap.map { "\(tab.name) \u{b7} \($0)" } ?? tab.name)
                .accessibilityLabel("\(tab.name), \(tab.count)")
                .accessibilityAddTraits(tab.on ? .isSelected : [])
            }
            Spacer(minLength: 12)
            Text(model.note)
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.head)
        }
        .padding(.horizontal, 20)
        .frame(height: 44)
    }

    // MARK: rows

    private var rows: some View {
        ScrollViewReader { scroller in
            ScrollView(.vertical) {
                LazyVStack(spacing: 0) {
                    ForEach(model.rows) { row in
                        if let heading = row.heading {
                            HStack {
                                Text(heading).font(.system(size: 12.5, weight: .bold))
                                Spacer()
                            }
                            .padding(.horizontal, 24)
                            .frame(height: Self.headingHeight)
                            .background(Color(nsColor: .windowBackgroundColor))
                            .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
                            .accessibilityAddTraits(.isHeader)
                        }
                        FilteredRowView(
                            row: row, focused: model.focused == row.id,
                            restore: model.restoreWords, restoreCap: model.restoreCap,
                            point: { model.point(row.id) }, open: { model.open(row.id) },
                            restoreAction: model.restore
                        )
                        .id(row.id)
                        .onAppear {
                            if row.id == model.rows.count - 1 { model.reachedEnd() }
                        }
                    }
                }
            }
            .onChange(of: model.focused) { _, focused in
                guard let focused else { return }
                scroller.scrollTo(focused)
            }
        }
    }
}

/// One filtered message: the sender, the subject and its first line, the
/// reason pill, the time. The focused row is ringed in the accent and
/// offers Restore with its key.
struct FilteredRowView: View {
    let row: FilteredModel.Row
    let focused: Bool
    let restore: String
    let restoreCap: String?
    let point: () -> Void
    let open: () -> Void
    let restoreAction: () -> Void

    var body: some View {
        HStack(spacing: 14) {
            Text(row.sender)
                .font(.system(size: 13.5))
                .lineLimit(1)
                .frame(width: FilteredView.senderWidth, alignment: .leading)
            HStack(spacing: 8) {
                Text(row.subject).font(.system(size: 13.5)).lineLimit(1).layoutPriority(1)
                if let preview = row.preview {
                    Text(preview).font(.system(size: 13)).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            if focused, !restore.isEmpty {
                FocusVerbButton(restore, cap: restoreCap, action: restoreAction)
                    .background(
                        RoundedRectangle(cornerRadius: 6).fill(Color(nsColor: .controlBackgroundColor)))
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
            }
            FocusChip(row.pill)
                .frame(width: 180, alignment: .leading)
            Text(row.time)
                .font(.system(size: 12.5).monospacedDigit())
                .foregroundStyle(.secondary)
                .frame(width: 48, alignment: .trailing)
        }
        .padding(.leading, 56)
        .padding(.trailing, 24)
        .frame(height: FilteredView.rowHeight)
        .background(focused ? Color.accentColor.opacity(0.08) : Color.clear)
        .overlay {
            if focused {
                Rectangle().strokeBorder(Color.accentColor, lineWidth: 2)
            }
        }
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
        .contentShape(Rectangle())
        .onTapGesture(count: 2, perform: open)
        .simultaneousGesture(TapGesture().onEnded(point))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(row.sender), \(row.subject), \(row.pill), \(row.time)")
        .accessibilityAddTraits(focused ? .isSelected : [])
    }
}
