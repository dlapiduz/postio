import PostioFFI
import SwiftUI

/// What the page an empty list shows says (specs/009-focus-macos T099,
/// screen 16): the controller's `EmptyPageFfi`, laid out.
///
/// The heading, the lines under it and the shortcuts are the controller's
/// (`postio_ui::focus_state::EmptyInbox`), and it offers only shortcuts that
/// lead somewhere: Filtered while Focus files mail away, Archive once a
/// pass has finished, Compose always. Here they become keycaps.
public struct EmptyInboxWords: Equatable, Sendable {
    /// One shortcut: its keycap, its words, the command a click runs.
    public struct Shortcut: Equatable, Sendable {
        public let cap: String?
        public let words: String
        public let command: String
    }

    public let heading: String
    /// The detail, then the next digest, each when there is one.
    public let lines: [String]
    public let shortcuts: [Shortcut]

    public init(_ page: EmptyPageFfi) {
        heading = page.heading
        lines = [page.detail, page.nextDigest].compactMap { $0 }
        shortcuts = page.shortcuts.map {
            Shortcut(cap: KeyCapSpelling.cap($0.key), words: $0.words, command: $0.command)
        }
    }
}

/// The empty page, drawn in the list's place: a quiet centred message with
/// the tray symbol, the lines, and the shortcuts as keycaps before their
/// words (screen 16). A click runs the shortcut's command.
public struct EmptyInbox: View {
    let words: EmptyInboxWords
    let run: (String) -> Void

    public init(words: EmptyInboxWords, run: @escaping (String) -> Void) {
        self.words = words
        self.run = run
    }

    public var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "tray")
                .font(.system(size: 26, weight: .regular))
                .foregroundStyle(.secondary)
                .padding(.bottom, 2)
                .accessibilityHidden(true)
            Text(words.heading)
                .font(.system(size: 17, weight: .bold))
            ForEach(words.lines, id: \.self) { line in
                Text(line)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
            }
            if !words.shortcuts.isEmpty {
                HStack(spacing: 14) {
                    ForEach(words.shortcuts, id: \.command) { shortcut in
                        Button { run(shortcut.command) } label: {
                            HStack(spacing: 5) {
                                if let cap = shortcut.cap { KeyCap(cap) }
                                Text(shortcut.words)
                                    .font(.system(size: 13))
                                    .foregroundStyle(.secondary)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(
                            shortcut.cap.map { "\(shortcut.words), key \($0)" } ?? shortcut.words)
                    }
                }
                .padding(.top, 4)
            }
        }
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.background)
        .accessibilityElement(children: .contain)
    }
}
