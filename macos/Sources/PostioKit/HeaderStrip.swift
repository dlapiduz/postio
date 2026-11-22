import Foundation
import PostioFFI
import SwiftUI

public struct HeaderStripWords: Equatable, Sendable {
    public enum Command {
        public static let folders = "go_to_folders"
        public static let hasAction = "toggle_has_action"
        public static let filtered = "go_to_filtered"
        public static let digestRules = "go_to_digest_rules"
    }

    public let place: String
    public let placeCap: String?
    public let counts: String?
    public let hasAction: String
    public let hasActionCap: String?
    public let hasActionOn: Bool
    public let filtered: String?
    public let filteredCap: String?
    public let digestRules: String?
    public let digestRulesCap: String?

    public init(
        strip: FocusStripFfi?, place: String = "Inbox", hasActionOn: Bool,
        binding: (String) -> String?
    ) {
        self.place = place
        placeCap = KeyCapSpelling.cap(binding(Command.folders))
        counts = strip?.counts
        hasAction = strip?.hasAction ?? "Has action"
        hasActionCap = KeyCapSpelling.cap(binding(Command.hasAction))
        self.hasActionOn = hasActionOn
        filtered = strip?.filteredToday
        filteredCap = KeyCapSpelling.cap(binding(Command.filtered))
        digestRules = strip?.digestRules
        digestRulesCap = KeyCapSpelling.cap(binding(Command.digestRules))
    }
}

/// `placeAnchor` is drawn behind Inbox ▾: the AppKit layer puts a view there for the folders
/// popover to hang from, which this target cannot name.
/// Semantic colours only; the accent is the toggle's, and only while it is on.
public struct HeaderStrip<PlaceAnchor: View>: View {
    let words: HeaderStripWords
    let run: (String) -> Void
    let placeAnchor: PlaceAnchor

    public init(words: HeaderStripWords, placeAnchor: PlaceAnchor, run: @escaping (String) -> Void) {
        self.words = words
        self.placeAnchor = placeAnchor
        self.run = run
    }

    public static var height: CGFloat { 36 }

    public var body: some View {
        HStack(spacing: 10) {
            Button { run(HeaderStripWords.Command.folders) } label: {
                HStack(spacing: 4) {
                    Text(words.place).font(.system(size: 15, weight: .bold))
                    Image(systemName: "chevron.down")
                        .font(.system(size: 9, weight: .semibold))
                }
                .foregroundStyle(.primary)
            }
            .buttonStyle(.plain)
            .background(placeAnchor)
            .accessibilityLabel("\(words.place), folders")
            if let cap = words.placeCap { KeyCap(cap) }
            if let counts = words.counts {
                Text(counts)
                    .font(.system(size: 13).monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            Divider().frame(height: 16)
            Button { run(HeaderStripWords.Command.hasAction) } label: {
                HStack(spacing: 6) {
                    Image(systemName: "flag").font(.system(size: 11))
                    Text(words.hasAction).font(.system(size: 13, weight: .bold).monospacedDigit())
                    if let cap = words.hasActionCap { KeyCap(cap) }
                }
                .foregroundStyle(words.hasActionOn ? AnyShapeStyle(.tint) : AnyShapeStyle(.primary))
                .padding(.horizontal, 8)
                .frame(height: 24)
                .background(
                    RoundedRectangle(cornerRadius: 6).fill(
                        words.hasActionOn ? AnyShapeStyle(.tint.opacity(0.12)) : AnyShapeStyle(.quaternary))
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(.tint, lineWidth: 1)
                        .opacity(words.hasActionOn ? 1 : 0)
                )
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(words.hasActionOn ? .isSelected : [])
            Spacer(minLength: 12)
            if let filtered = words.filtered {
                Button { run(HeaderStripWords.Command.filtered) } label: {
                    HStack(spacing: 5) {
                        Text(filtered).font(.system(size: 13).monospacedDigit())
                        if let cap = words.filteredCap { KeyCap(cap) }
                    }
                    .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
            if let rules = words.digestRules {
                Button { run(HeaderStripWords.Command.digestRules) } label: {
                    HStack(spacing: 5) {
                        Text(rules).font(.system(size: 13).monospacedDigit())
                        if let cap = words.digestRulesCap { KeyCap(cap) }
                    }
                    .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 20)
        .frame(height: Self.height)
        .frame(maxWidth: .infinity)
        .background(.background)
        .overlay(alignment: .bottom) { Divider() }
    }
}

extension HeaderStrip where PlaceAnchor == EmptyView {
    public init(words: HeaderStripWords, run: @escaping (String) -> Void) {
        self.init(words: words, placeAnchor: EmptyView(), run: run)
    }
}
