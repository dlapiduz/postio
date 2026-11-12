import PostioFFI
import SwiftUI

// The message window's thread bar (spec 013 US3, T047; the Mac threads
// handoff SPEC section 4, screens 02 and 03): 60pt tall under the action row
// with a hairline below it, a 66pt label column, one 72x46 chip per message
// and the Thread button on the right.
//
// The controller decides which chips fit and which is current
// (`postio_ui::thread_bar`, `Intent::ThreadBar`); this draws what it is told.
// The accent is for the current chip's ring and the unread dots, nothing else.

/// The bar's measures, in points.
public enum ThreadBarMetrics {
    public static let height: CGFloat = 60
    public static let labelColumn: CGFloat = 66
    public static let chip = CGSize(width: 72, height: 46)
    public static let chipRadius: CGFloat = 8
    public static let chipGap: CGFloat = 4
    public static let avatar: CGFloat = 20
    /// The padding either side of the bar's contents.
    public static let edge: CGFloat = 16
    /// What the Thread button, and the gap before it, take of the width.
    public static let button: CGFloat = 88
    public static let buttonGap: CGFloat = 8

    /// How many chips a window `width` wide has room for, after the label
    /// column and the Thread button: what `Input::BarRoom` carries. Always at
    /// least one, so a narrow window still draws the message on screen.
    public static func room(window width: CGFloat) -> UInt32 {
        let free = width - 2 * edge - labelColumn - buttonGap - button - buttonGap
        let each = chip.width + chipGap
        return UInt32(max(1, ((free + chipGap) / each).rounded(.down)))
    }
}

/// The bar. Nothing draws for a thread of one message.
public struct ThreadBarView: View {
    let bar: ThreadBarFfi
    let menu: () -> Void

    /// `menu` opens Thread ▾'s menu: the command the key runs.
    public init(bar: ThreadBarFfi, menu: @escaping () -> Void = {}) {
        self.bar = bar
        self.menu = menu
    }

    public var body: some View {
        HStack(alignment: .center, spacing: 0) {
            VStack(alignment: .leading, spacing: 1) {
                Text(bar.title)
                    .font(.system(size: 11.5, weight: .bold))
                    .foregroundStyle(.primary)
                Text(bar.countLine)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .frame(width: ThreadBarMetrics.labelColumn, alignment: .leading)
            HStack(spacing: ThreadBarMetrics.chipGap) {
                ForEach(Array(bar.chips.enumerated()), id: \.offset) { _, chip in
                    ThreadChipView(chip: chip)
                }
            }
            .padding(.leading, ThreadBarMetrics.buttonGap)
            Spacer(minLength: ThreadBarMetrics.buttonGap)
            ThreadMenuButton(label: bar.menuLabel, action: menu)
        }
        .padding(.horizontal, ThreadBarMetrics.edge)
        .frame(height: ThreadBarMetrics.height)
        .frame(maxWidth: .infinity)
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(bar.title), \(bar.countLine)")
    }
}

/// One chip: an avatar over "Name · time".
struct ThreadChipView: View {
    let chip: ThreadChipFfi

    var body: some View {
        switch chip {
        case let .message(_, initials, label, current, unread, matched):
            message(initials: initials, label: label, current: current, unread: unread, matched: matched)
        case let .more(count, _, _):
            more(count)
        }
    }

    private func message(initials: String, label: String, current: Bool, unread: Bool, matched: Bool)
        -> some View
    {
        let shape = RoundedRectangle(cornerRadius: ThreadBarMetrics.chipRadius)
        return VStack(spacing: 3) {
            Text(initials)
                .font(.system(size: 8.5, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: ThreadBarMetrics.avatar, height: ThreadBarMetrics.avatar)
                .background(Circle().fill(.secondary))
            Text(label)
                .font(.system(size: 10.5, weight: current ? .semibold : .regular))
                .foregroundStyle(current ? .primary : .secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .padding(.horizontal, 3)
        }
        .frame(width: ThreadBarMetrics.chip.width, height: ThreadBarMetrics.chip.height)
        .background(shape.fill(current ? Color.accentColor.opacity(0.12) : .clear))
        .overlay(shape.strokeBorder(current ? Color.accentColor : .clear, lineWidth: 2))
        .overlay(alignment: .topTrailing) {
            if unread {
                Circle().fill(Color.accentColor).frame(width: 6, height: 6).padding(5)
            }
        }
        .overlay(alignment: .topLeading) {
            if matched {
                Rectangle().fill(.yellow).frame(width: 6, height: 6).padding(5)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            [label, current ? "shown" : nil, unread ? "unread" : nil, matched ? "matches" : nil]
                .compactMap { $0 }.joined(separator: ", "))
        .accessibilityAddTraits(current ? .isSelected : [])
    }

    private func more(_ count: UInt32) -> some View {
        Text("\u{2026} \(count) more")
            .font(.system(size: 10.5))
            .foregroundStyle(.secondary)
            .frame(width: ThreadBarMetrics.chip.width, height: ThreadBarMetrics.chip.height)
            .accessibilityLabel("\(count) more messages")
    }
}

/// "Thread ▾", on the right of the bar, outlined.
struct ThreadMenuButton: View {
    let label: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Text(label).font(.system(size: 13))
                Image(systemName: "chevron.down").font(.system(size: 9, weight: .semibold))
            }
            .foregroundStyle(.primary)
            .padding(.horizontal, 10)
            .frame(width: ThreadBarMetrics.button, height: 28)
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
        // Where it is, for the menu and the outline to hang from.
        .background(
            GeometryReader { proxy in
                Color.clear.preference(
                    key: MessageVerbFrames.self,
                    value: [ThreadMenuCommand.menu: proxy.frame(in: .named(MessageVerbFrames.space))])
            })
    }
}

/// The quiet banner on an older message: its words, and "Latest }".
struct ThreadBannerView: View {
    let banner: ThreadBannerFfi
    /// A new reply is tinted with the accent (screen 08); the older-message
    /// banner is quiet.
    var tinted = false
    let run: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Text(banner.text)
                .font(.system(size: 12.5))
                .foregroundStyle(tinted ? .primary : .secondary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button(action: run) {
                HStack(spacing: 5) {
                    Text(banner.action).font(.system(size: 12.5, weight: .medium))
                    if let key = banner.key { KeyCap(key) }
                }
                .foregroundStyle(.primary)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(
            RoundedRectangle(cornerRadius: 8).fill(
                tinted ? AnyShapeStyle(Color.accentColor.opacity(0.12)) : AnyShapeStyle(.quaternary.opacity(0.6))))
        .accessibilityElement(children: .contain)
    }
}

/// The banner of a window opened from a search (spec 013 US6; screen 07):
/// the search's words marked, where the message on screen is among the
/// thread's matches, and the two ways on, each with its key. The marked
/// words wear the find yellow, never the accent.
struct MatchBannerView: View {
    let banner: MatchBannerFfi
    let next: () -> Void
    let back: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            Text(words)
                .font(.system(size: 12.5))
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            button(banner.next, key: banner.nextKey, action: next)
            button(banner.back, key: banner.backKey, action: back)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.6)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(banner.text)
    }

    /// The text with the quoted query marked and its quotes dropped.
    private var words: AttributedString {
        let text = banner.text
        guard text.hasPrefix("\u{2018}"), let close = text.firstIndex(of: "\u{2019}") else {
            return AttributedString(text)
        }
        var query = AttributedString(String(text[text.index(after: text.startIndex)..<close]))
        query.backgroundColor = SearchRuns.highlight(scheme)
        query.foregroundColor = .primary
        var rest = AttributedString(String(text[text.index(after: close)...]))
        rest.foregroundColor = .secondary
        return query + rest
    }

    private func button(_ label: String, key: String?, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Text(label).font(.system(size: 12.5, weight: .medium))
                if let key { KeyCap(KeyCapSpelling.cap(key) ?? key) }
            }
            .foregroundStyle(.primary)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}
