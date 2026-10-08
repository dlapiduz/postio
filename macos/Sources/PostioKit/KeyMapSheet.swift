import Observation
import PostioFFI
import SwiftUI

/// What the key map says (`?`, screen 20; specs/009-focus-macos T106): the
/// controller's `KeyMapSheetFfi`, laid out.
///
/// The groups, their rows and titles, the columns they stand in and both
/// footers are `postio_ui::keymap_sheet`'s for Focus on Apple -- the key map
/// GTK draws, less what the Mac does not offer -- and the footer names
/// `~/Library/Application Support/Postio/config.toml` and `[keys]` (C3).
/// What is decided here is only spelling: every key as the menus spell it
/// (`KeyCapSpelling`), a row's bindings in one cap as the pack draws them.
public struct KeyMapSheetWords: Equatable, Sendable {
    /// One command: its words and its keys.
    public struct Row: Equatable, Sendable {
        public let command: String
        public let title: String
        /// Every binding, spelled, in one cap; `nil` with none.
        public let cap: String?
    }

    /// One group, kept whole in its column.
    public struct Group: Equatable, Sendable {
        public let title: String
        public let rows: [Row]
    }

    public let title: String
    public let subtitle: String
    /// The keys that close it (`?`, Escape), each a cap.
    public let closeCaps: [String]
    public let closeOr: String
    public let closeWord: String
    /// The groups, column by column, as `columns` names them.
    public let columns: [[Group]]
    /// "Rebind anything in … under [keys]".
    public let rebind: String
    /// "The mouse works everywhere: …".
    public let mouse: String

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
        // An index past the groups is skipped rather than trusted: a column
        // that named a group this build does not have draws without it.
        columns = sheet.columns.map { column in
            column.compactMap { index in
                Int(index) < groups.count ? groups[Int(index)] : nil
            }
        }
        rebind = sheet.rebindFooter
        mouse = sheet.mouseFooter
    }
}

/// Whether the key map is up, and what it says, as the controller's events
/// leave it. The controller puts it on its stack and takes it off, so the
/// Mac reports neither -- only a close of its own, a click outside it.
@MainActor
@Observable
public final class KeyMapModel {
    /// What an applied event did.
    public enum Change: Equatable, Sendable {
        case open
        case close
    }

    /// What the open key map draws; `nil` while it is closed.
    public private(set) var words: KeyMapSheetWords?

    public var isOpen: Bool { words != nil }

    public init() {}

    /// `FocusOpenKeyMap` opens it and `FocusCloseSurface(.keyMap)` closes
    /// it; every other event is `nil` and changes nothing.
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

    /// Draw `sheet` in place of what is up -- after `[keys]` changed. A
    /// closed key map stays closed.
    public func refresh(_ sheet: KeyMapSheetFfi) {
        guard isOpen else { return }
        words = KeyMapSheetWords(sheet)
    }

    /// Closed by the toolkit (a click outside): whether it was up, so the
    /// controller is told once.
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        words = nil
        return true
    }
}

/// The key map, drawn (screen 20): a panel centred over the dimmed main
/// window -- the title and the subtitle, the close keys on the right, the
/// groups in four columns of rows with their caps, and the two footers
/// under a hairline. A click on the dim closes it.
public struct KeyMapPanel: View {
    let words: KeyMapSheetWords
    let dismiss: () -> Void

    public init(words: KeyMapSheetWords, dismiss: @escaping () -> Void) {
        self.words = words
        self.dismiss = dismiss
    }

    /// The pack's panel at 1440 × 900.
    public static let size = CGSize(width: 1100, height: 760)

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
            // Every command Focus offers here is in it -- far more rows than
            // the pack's picture -- so the columns scroll inside the panel,
            // and the heading and the footers stay put.
            ScrollView {
                HStack(alignment: .top, spacing: 28) {
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
            }
            .scrollIndicators(.automatic)
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

/// One group: its heading, then a row per command, title on the left and
/// its cap on the right, a hairline under each.
private struct KeyMapGroupView: View {
    let group: KeyMapSheetWords.Group

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(group.title)
                .font(.system(size: 13, weight: .bold))
                .padding(.bottom, 6)
            ForEach(group.rows, id: \.command) { row in
                HStack(alignment: .center, spacing: 8) {
                    Text(row.title)
                        .font(.system(size: 13))
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    if let cap = row.cap { KeyCap(cap) }
                }
                .frame(minHeight: 30)
                .overlay(alignment: .bottom) { Divider() }
                .accessibilityElement(children: .combine)
                .accessibilityLabel(row.cap.map { "\(row.title), \($0)" } ?? row.title)
            }
        }
    }
}
