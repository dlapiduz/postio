import PostioFFI
import SwiftUI

public struct FilteredView: View {
    let model: FilteredModel
    let scope: FooterViewFfi?

    public init(model: FilteredModel, scope: FooterViewFfi? = nil) {
        self.model = model
        self.scope = scope
    }

    public static let rowHeight: CGFloat = 40
    public static let headingHeight: CGFloat = 32
    public static let senderWidth: CGFloat = 196

    public var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            tabs
            Divider()
            rows
            if let bar = model.bar {
                ActionBar(words: bar, run: model.run)
                    .transition(.opacity)
            }
            if let scope {
                ScopeFooter(words: ScopeFooterWords(scope))
            } else {
                Divider()
                FocusHintRow(model.footer.map { FocusHintRow.Hint(cap: $0.cap, label: $0.label) })
                    .padding(.horizontal, 20)
                    .frame(height: 30)
                    .background(.bar)
            }
        }
        .background(Color(nsColor: .textBackgroundColor))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(model.title)
    }

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

    private var rows: some View {
        ScrollViewReader { scroller in
            if let empty = model.empty {
                Text(empty)
                    .font(.system(size: 13.5))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityAddTraits(.updatesFrequently)
            }
            ScrollView(.vertical) {
                LazyVStack(spacing: 0) {
                    ForEach(model.items) { item in
                        switch item {
                        case let .row(row, display):
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
                                row: row, focused: model.focused == display,
                                expanded: model.open?.anchor == row.id,
                                restore: model.bar == nil ? model.restoreWords : "", restoreCap: model.restoreCap,
                                point: { model.point(display) }, open: { model.open(display) },
                                toggle: { model.toggle(display) }, extend: { model.extend(display) },
                                restoreAction: model.restore
                            )
                            .id(display)
                            .onAppear {
                                if row.id == model.rows.count - 1 { model.reachedEnd() }
                            }
                        case let .child(child, display, last):
                            FilteredChildView(
                                child: child, focused: model.focused == display, last: last,
                                point: { model.point(display) }, open: { model.open(display) },
                                toggle: { model.toggle(display) }, extend: { model.extend(display) }
                            )
                            .id(display)
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

struct FilteredRowView: View {
    let row: FilteredModel.Row
    let focused: Bool
    let expanded: Bool
    let restore: String
    let restoreCap: String?
    let point: () -> Void
    let open: () -> Void
    let toggle: () -> Void
    let extend: () -> Void
    let restoreAction: () -> Void

    var body: some View {
        HStack(spacing: 14) {
            Text(row.sender)
                .font(.system(size: 13.5))
                .lineLimit(1)
                .frame(width: FilteredView.senderWidth, alignment: .leading)
            HStack(spacing: 8) {
                Text(row.subject).font(.system(size: 13.5)).lineLimit(1).layoutPriority(1)
                if row.isThread {
                    Text("\(row.messages)")
                        .font(.system(size: 11.5).monospacedDigit())
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 5)
                        .background(RoundedRectangle(cornerRadius: 4).fill(Color.primary.opacity(0.08)))
                }
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
        // A neutral ground for the selection, never the accent, which is the cursor's.
        .background(row.selected ? Color.primary.opacity(0.06) : Color.clear)
        .background(focused ? Color.accentColor.opacity(0.08) : Color.clear)
        .overlay(alignment: .leading) {
            if row.selected {
                Image(systemName: "checkmark.square.fill")
                    .font(.system(size: 14))
                    .frame(width: 14, height: 14)
                    .position(x: FocusRowMetrics.gutterCentre, y: FilteredView.rowHeight / 2)
                    .accessibilityHidden(true)
            } else if row.isThread {
                Image(systemName: "chevron.right")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
                    .position(x: FocusRowMetrics.gutterCentre, y: FilteredView.rowHeight / 2)
                    .accessibilityHidden(true)
            }
        }
        .overlay {
            if focused {
                Rectangle().strokeBorder(Color.accentColor, lineWidth: 2)
            }
        }
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
        .contentShape(Rectangle())
        // The modifier decides in SwiftUI, so a ⌘-click is never also a point or an open.
        .gesture(TapGesture(count: 2).modifiers([]).onEnded(open))
        .gesture(TapGesture().modifiers([]).onEnded(point))
        .gesture(TapGesture().modifiers(.command).onEnded(toggle))
        .gesture(TapGesture().modifiers(.shift).onEnded(extend))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "\(row.selected ? "Selected, " : "")\(row.sender), \(row.subject), "
                + "\(row.isThread ? "\(row.messages) messages, \(expanded ? "open" : "closed"), " : "")"
                + "\(row.pill), \(row.time)"
        )
        .accessibilityAddTraits(focused ? .isSelected : [])
    }
}

struct FilteredChildView: View {
    let child: FilteredModel.Child
    let focused: Bool
    let last: Bool
    let point: () -> Void
    let open: () -> Void
    let toggle: () -> Void
    let extend: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Text(child.initials)
                .font(.system(size: 10, weight: .semibold))
                .frame(width: CGFloat(FocusRowMetrics.childAvatar), height: CGFloat(FocusRowMetrics.childAvatar))
                .background(Circle().fill(Color(nsColor: .tertiaryLabelColor)))
            Text(child.sender)
                .font(.system(size: 13.5, weight: child.unread ? .bold : .regular))
                .italic(child.draft)
                .lineLimit(1)
                .frame(width: FilteredView.senderWidth - 70, alignment: .leading)
            if let tag = child.tag, !child.draft {
                Text(tag)
                    .font(.system(size: 11.5))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 6)
                    .background(RoundedRectangle(cornerRadius: 4).fill(Color(nsColor: .quaternaryLabelColor)))
            }
            Text(words)
                .font(.system(size: 13.5))
                .italic(child.draft || child.placeholder)
                .foregroundStyle(child.placeholder ? .tertiary : .secondary)
                .lineLimit(1)
            Spacer(minLength: 0)
            if focused, let label = child.hintLabel, !child.hintKeys.isEmpty {
                HStack(spacing: 4) {
                    ForEach(child.hintKeys, id: \.self) { KeyCap($0) }
                    Text(label).font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }
            if child.attachment {
                Image(systemName: "paperclip").font(.system(size: 12)).foregroundStyle(.secondary)
            }
            Text(child.time)
                .font(.system(size: 12.5, weight: child.unread ? .semibold : .regular).monospacedDigit())
                .foregroundStyle(child.draft ? .secondary : .primary)
                .frame(width: 48, alignment: .trailing)
        }
        .padding(.leading, CGFloat(FocusRowMetrics.childAvatarX))
        .padding(.trailing, 24)
        .frame(height: CGFloat(FocusRowMetrics.child))
        .background(focused ? Color.accentColor.opacity(0.08) : Color.clear)
        .overlay(alignment: .leading) {
            Rectangle()
                .fill(child.unread ? Color.accentColor : Color(nsColor: .tertiaryLabelColor))
                .frame(width: CGFloat(FocusRowMetrics.childLineWidth), height: last ? CGFloat(FocusRowMetrics.child) / 2 : CGFloat(FocusRowMetrics.child))
                .frame(maxHeight: .infinity, alignment: .top)
                .padding(.leading, CGFloat(FocusRowMetrics.childLineX))
        }
        .overlay {
            if focused {
                Rectangle().strokeBorder(Color.accentColor, lineWidth: 2)
            }
        }
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
        .contentShape(Rectangle())
        .gesture(TapGesture(count: 2).modifiers([]).onEnded(open))
        .gesture(TapGesture().modifiers([]).onEnded(point))
        .gesture(TapGesture().modifiers(.command).onEnded(toggle))
        .gesture(TapGesture().modifiers(.shift).onEnded(extend))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "\(child.sender), \(child.tag.map { "\($0), " } ?? "")\(child.line), \(child.time)"
                + (child.unread ? ", unread" : "") + (child.draft ? ", draft" : "")
        )
        .accessibilityAddTraits(focused ? .isSelected : [])
    }

    private var words: String {
        child.draft
            ? "\u{201c}\(child.line)\u{201d}" + (child.tag.map { " \u{b7} \($0)" } ?? "")
            : child.line
    }
}
