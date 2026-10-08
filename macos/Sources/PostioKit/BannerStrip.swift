import PostioFFI
import SwiftUI

/// What the banner under the header strip says (specs/009-focus-macos
/// T099, screens 17 to 19): the controller's `BannerFfi`, laid out.
///
/// Every word is the controller's (`postio_ui::focus_state::Banner`): the
/// bold heading, the sentence, the button and the command it runs. What is
/// decided here is only drawing: the button's keycap spelled as every
/// other cap is (`KeyCapSpelling`), the progress as a fraction, and the
/// tone.
public struct BannerStripWords: Equatable, Sendable {
    /// How the strip is filled: plain, or `systemRed` at a low opacity.
    public enum Tone: Equatable, Sendable {
        case plain
        case error
    }

    /// The strip's button: "Retry now", "Update password…".
    public struct Button: Equatable, Sendable {
        public let label: String
        /// The registry command a click runs, as a menu item would.
        public let command: String
        /// Its keycap, or `nil` when nothing is bound to it.
        public let cap: String?
    }

    public let heading: String
    public let sentence: String
    public let button: Button?
    /// How far a first sync has come, from 0 to 1; `nil` draws no bar.
    public let progress: Double?
    public let tone: Tone
    /// The account a refused password is for: whose credential "Update
    /// password…" asks for.
    public let account: Int64?

    public init(_ banner: BannerFfi) {
        heading = banner.heading
        sentence = banner.sentence
        button = banner.button.map {
            Button(label: $0.label, command: $0.command, cap: KeyCapSpelling.cap($0.key))
        }
        progress = banner.progress.flatMap { progress in
            guard progress.total > 0 else { return nil }
            return min(1, Double(progress.done) / Double(progress.total))
        }
        tone = banner.error ? .error : .plain
        account = banner.account
    }
}

/// The banner, drawn: one full-width strip under the header strip, the
/// heading in bold, the sentence, a progress line while a first sync runs,
/// and the button with its keycap (screens 17 to 19). A click runs the
/// button's command through the path its key takes.
public struct BannerStrip: View {
    let words: BannerStripWords
    let run: (String) -> Void

    public init(words: BannerStripWords, run: @escaping (String) -> Void) {
        self.words = words
        self.run = run
    }

    /// The strip's height in the pack (screens 17 to 19).
    public static let height: CGFloat = 42

    public var body: some View {
        HStack(spacing: 14) {
            Text(words.heading)
                .font(.system(size: 13, weight: .bold))
                .foregroundStyle(.primary)
            Text(words.sentence)
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
            if let progress = words.progress {
                ProgressView(value: progress)
                    .progressViewStyle(.linear)
                    .frame(width: 180)
                    .accessibilityLabel("First sync")
            }
            if let button = words.button {
                Button { run(button.command) } label: {
                    HStack(spacing: 6) {
                        Text(button.label).font(.system(size: 13, weight: .semibold))
                        if let cap = button.cap { KeyCap(cap) }
                    }
                }
                .buttonStyle(.bordered)
                .controlSize(.regular)
                .accessibilityLabel(button.cap.map { "\(button.label), key \($0)" } ?? button.label)
            }
        }
        .padding(.horizontal, 20)
        .frame(maxWidth: .infinity)
        .frame(height: Self.height)
        .background(fill)
        .overlay(alignment: .bottom) { Divider() }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(words.heading)
    }

    /// `systemRed` at a low opacity for an error (contracts/mac-surfaces.md
    /// "Colour"), otherwise the window's own band, a shade off the list.
    @ViewBuilder private var fill: some View {
        switch words.tone {
        case .error: Rectangle().fill(Color.red.opacity(0.1))
        case .plain: Rectangle().fill(.quinary)
        }
    }
}
