import PostioFFI
import SwiftUI

public struct EmptyInboxWords: Equatable, Sendable {
    public struct Shortcut: Equatable, Sendable {
        public let cap: String?
        public let words: String
        public let command: String
    }

    public let heading: String
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
