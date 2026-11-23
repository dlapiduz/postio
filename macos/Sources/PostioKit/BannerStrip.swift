import PostioFFI
import SwiftUI

public struct BannerStripWords: Equatable, Sendable {
    public enum Tone: Equatable, Sendable {
        case plain
        case error
    }

    public struct Button: Equatable, Sendable {
        public let label: String
        public let command: String
        public let cap: String?
    }

    public let heading: String
    public let sentence: String
    public let button: Button?
    public let progress: Double?
    public let tone: Tone
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

public struct BannerStrip: View {
    let words: BannerStripWords
    let run: (String) -> Void

    public init(words: BannerStripWords, run: @escaping (String) -> Void) {
        self.words = words
        self.run = run
    }

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

    @ViewBuilder private var fill: some View {
        switch words.tone {
        case .error: Rectangle().fill(Color.red.opacity(0.1))
        case .plain: Rectangle().fill(.quinary.opacity(0.5))
        }
    }
}
