import Observation
import PostioFFI
import SwiftUI

/// The footer names `~/Library/Application Support/Postio/config.toml` and `[keys]` (C3). Only
/// spelling is decided here: every key as the menus spell it (`KeyCapSpelling`).
public struct KeyMapSheetWords: Equatable, Sendable {
    public struct Row: Equatable, Sendable {
        public let command: String
        public let title: String
        public let cap: String?
    }

    public struct Group: Equatable, Sendable {
        public let title: String
        public let rows: [Row]
    }

    public let title: String
    public let subtitle: String
    public let closeCaps: [String]
    public let closeOr: String
    public let closeWord: String
    public let columns: [[Group]]
    public let rebind: String
    public let mouse: String
    public let more: String

    public init(_ sheet: KeyMapSheetFfi) {
        title = sheet.title
        subtitle = sheet.subtitle
        closeCaps = sheet.closeKeys.compactMap(KeyCapSpelling.cap)
        closeOr = sheet.closeOr
        closeWord = sheet.closeWord
        let groups = sheet.groups.map { group in
            Group(
                title: group.title,
                rows: group.rows.map { row in
                    let caps = row.keys.compactMap(KeyCapSpelling.cap)
                    return Row(
                        command: row.command, title: row.title,
                        cap: caps.isEmpty ? nil : caps.joined(separator: " "))
                })
        }
        // An index past the groups is skipped rather than trusted.
        columns = sheet.columns.map { column in
            column.compactMap { index in
                Int(index) < groups.count ? groups[Int(index)] : nil
            }
        }
        rebind = sheet.rebindFooter
        mouse = sheet.mouseFooter
        more = sheet.moreBelow
    }
}

/// The controller puts it on its stack and takes it off, so the Mac reports neither, only a close
/// of its own (a click outside).
@MainActor
@Observable
public final class KeyMapModel {
    public enum Change: Equatable, Sendable {
        case open
        case close
    }

    public private(set) var words: KeyMapSheetWords?

    public var isOpen: Bool { words != nil }

    public init() {}

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusOpenKeyMap(sheet):
            words = KeyMapSheetWords(sheet)
            return .open
        case .focusCloseSurface(.keyMap):
            words = nil
            return .close
        default:
            return nil
        }
    }

    public func refresh(_ sheet: KeyMapSheetFfi) {
        guard isOpen else { return }
        words = KeyMapSheetWords(sheet)
    }

    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        words = nil
        return true
    }
}

public struct KeyMapPanel: View {
    let words: KeyMapSheetWords
    let dismiss: () -> Void

    public init(words: KeyMapSheetWords, dismiss: @escaping () -> Void) {
        self.words = words
        self.dismiss = dismiss
    }

    public static let size = CGSize(width: 1100, height: 760)

    static func moreBelow(contentBottom: CGFloat, viewport: CGFloat) -> Bool {
        contentBottom > viewport + 1
    }

    static let fadeHeight: CGFloat = 56

    @State private var contentBottom: CGFloat = 0
    @State private var viewport: CGFloat = 0

    public var body: some View {
        ZStack {
            Rectangle()
                .fill(Color.black.opacity(0.25))
                .contentShape(Rectangle())
                .onTapGesture(perform: dismiss)
                .accessibilityHidden(true)
            panel
                .frame(maxWidth: Self.size.width, maxHeight: Self.size.height)
                .padding(.horizontal, 24)
                .padding(.vertical, 18)
        }
    }

    private var isScrollable: Bool { contentBottom > viewport + 1 && viewport > 0 }

    private func moreCue(_ scroll: @escaping () -> Void) -> some View {
        VStack(spacing: 0) {
            Rectangle()
                .fill(.background)
                .mask(LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom))
                .frame(height: Self.fadeHeight)
                .allowsHitTesting(false)
            Button(action: scroll) {
                HStack(spacing: 4) {
                    Image(systemName: "chevron.down").font(.system(size: 10, weight: .semibold))
                    Text(words.more)
                }
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 4)
                .background(.background)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .transition(.opacity)
    }

    private var panel: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Text(words.title).font(.system(size: 22, weight: .bold))
                Text(words.subtitle)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 16)
                HStack(spacing: 5) {
                    ForEach(Array(words.closeCaps.enumerated()), id: \.offset) { index, cap in
                        if index > 0 {
                            Text(words.closeOr).font(.system(size: 13)).foregroundStyle(.secondary)
                        }
                        KeyCap(cap)
                    }
                    Text(words.closeWord).font(.system(size: 13)).foregroundStyle(.secondary)
                }
                .fixedSize()
            }
            .padding(.bottom, 22)
            // The columns scroll inside the panel while the heading and footers stay put. A Mac
            // hides an idle scroller, so while rows
            // lie below the scroller stays drawn and the last rows fade under a "more below" line.
            ScrollViewReader { reader in
                ScrollView {
                    HStack(alignment: .top, spacing: 24) {
                        ForEach(Array(words.columns.enumerated()), id: \.offset) { _, column in
                            VStack(alignment: .leading, spacing: 22) {
                                ForEach(column, id: \.title) { group in
                                    KeyMapGroupView(group: group)
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .topLeading)
                        }
                    }
                    .padding(.bottom, 16)
                    .background(
                        GeometryReader { content in
                            Color.clear.preference(
                                key: KeyMapContentBottom.self,
                                value: content.frame(in: .named(KeyMapContentBottom.space)).maxY)
                        })
                    Color.clear.frame(height: 0).id(KeyMapContentBottom.end)
                }
                .coordinateSpace(name: KeyMapContentBottom.space)
                .scrollIndicators(isScrollable ? .visible : .automatic)
                .background(
                    GeometryReader { box in
                        Color.clear
                            .onAppear { viewport = box.size.height }
                            .onChange(of: box.size.height) { _, height in viewport = height }
                    })
                .onPreferenceChange(KeyMapContentBottom.self) { contentBottom = $0 }
                .overlay(alignment: .bottom) {
                    if Self.moreBelow(contentBottom: contentBottom, viewport: viewport) {
                        moreCue { withAnimation(.easeOut(duration: 0.1)) { reader.scrollTo(KeyMapContentBottom.end, anchor: .bottom) } }
                    }
                }
            }
            Divider()
            HStack {
                Text(words.rebind)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(.secondary)
                Spacer(minLength: 16)
                Text(words.mouse)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
            }
            .padding(.top, 12)
        }
        .padding(28)
        .background(.background, in: .rect(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(.separator, lineWidth: 1))
        .shadow(color: .black.opacity(0.2), radius: 24, y: 8)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(words.title)
    }
}

private struct KeyMapGroupView: View {
    let group: KeyMapSheetWords.Group

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(group.title)
                .font(.system(size: 13, weight: .bold))
                .padding(.bottom, 6)
            ForEach(group.rows, id: \.command) { row in
                HStack(alignment: .center, spacing: 8) {
                    // One line, so every row is 30 and the columns line up: a long title tightens
                    // to fit rather than wrap.
                    Text(row.title)
                        .font(.system(size: 13))
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                        .allowsTightening(true)
                    Spacer(minLength: 8)
                    if let cap = row.cap { KeyCap(cap).fixedSize() }
                }
                .frame(height: 30)
                .overlay(alignment: .bottom) { Divider() }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(row.cap.map { "\(row.title), \($0)" } ?? row.title)
            }
        }
    }
}

private struct KeyMapContentBottom: PreferenceKey {
    static let space = "keymap.scroll"
    static let end = "keymap.end"
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = max(value, nextValue())
    }
}
