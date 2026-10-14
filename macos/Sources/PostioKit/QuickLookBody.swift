import Foundation
import Observation
import PostioFFI
import SwiftUI

/// What Quick Look's buttons tell the engine (specs/010-focus-search
/// T091): the same commands their keys run.
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `QuickLookModel` without a store.
public protocol QuickLookEngine: AnyObject {
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
}

extension PostioSession: QuickLookEngine {}

/// Quick Look, as the controller's last `FocusQuickLook` drew it (design
/// §3.7, screen 10).
///
/// The controller (`crates/postio-focus/src/results.rs`) holds which
/// result it shows, its conversation's matches and which card is ringed;
/// Space, j/k, ]/[, ↩, `a` and Esc are its keys, answered there. Every
/// `FocusQuickLook` is the panel whole, and one while it is up is new
/// content for the same panel; `None` closes it. Its buttons run the
/// commands their keys do.
///
/// No AppKit (#1264): `QuickLookPanel` presents it and `QuickLookBody`
/// draws it.
@MainActor
@Observable
public final class QuickLookModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// It opened: put the panel up.
        case open
        /// The panel is up and shows something new: draw it in place.
        case redraw
        /// It closed.
        case close
    }

    /// What it shows; `nil` while it is closed.
    public private(set) var view: QuickLookViewFfi?

    @ObservationIgnored private let engine: QuickLookEngine

    public init(engine: QuickLookEngine) {
        self.engine = engine
    }

    /// Apply `event` if it is Quick Look's, and say what it changed; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        guard case let .focusQuickLook(next) = event else { return nil }
        guard let next else {
            guard view != nil else { return nil }
            view = nil
            return .close
        }
        let opened = view == nil
        view = next
        return opened ? .open : .redraw
    }

    /// Open ↩.
    public func open() { engine.invoke("open_message") }
    /// Archive a.
    public func archive() { engine.invoke("archive") }
    /// Close Space.
    public func close() { engine.invoke("quick_look") }
}

/// The few words Quick Look joins from the controller's (§3.7).
public enum QuickLookWords {
    /// The header's second phrase: "1 of 12 · j k moves through results
    /// while it stays open".
    public static func header(_ view: QuickLookViewFfi) -> String {
        guard let walk = view.walk else { return view.position }
        return "\(view.position) · \(inline(walk))"
    }

    /// After the match count: "· ] [ jump between them".
    public static func matchesHint(_ view: QuickLookViewFfi) -> String? {
        view.matchesHint.map { "· \(inline($0))" }
    }

    /// A pair of keys in running text, as the design writes them: "j k".
    static func inline(_ hint: KeyHintFfi) -> String {
        let keys = (KeyCapSpelling.cap(hint.key) ?? hint.key).replacingOccurrences(of: "/", with: " ")
        return "\(keys) \(hint.label)"
    }
}

/// Quick Look's sizes (§3.7).
public enum QuickLookMetrics {
    public static let width: CGFloat = 780
    public static let height: CGFloat = 470
    public static let radius: CGFloat = 14
    /// The panel's top, under the window's top: screen 10's 160, under the
    /// toolbar, the filter bar and the timeline.
    public static let top: CGFloat = 160
    public static let header: CGFloat = 46
    /// A card's where-and-when column.
    public static let placeColumn: CGFloat = 120
    /// The header's and a card's ground: a faint lift off the panel's
    /// white (#f3f3f3 / #f5f5f7 in screen 10), not the window's grey.
    public static let tint = Color.primary.opacity(0.045)
}

/// What Quick Look draws (specs/010-focus-search T091; design §3.7,
/// screen 10): the header -- "Quick Look", which result of how many and
/// the walk's keys, then Open, Archive and Close with theirs -- and the
/// body: the subject at 22/28 bold with its words marked, the sender line,
/// how many matches the conversation holds, and one card per match with
/// where and when in a 120 column and the passage at 14/22, the current
/// one ringed in the accent.
public struct QuickLookBody: View {
    let model: QuickLookModel
    @Environment(\.colorScheme) private var scheme

    public init(model: QuickLookModel) {
        self.model = model
    }

    public var body: some View {
        Group {
            if let view = model.view {
                VStack(spacing: 0) {
                    header(view)
                    content(view)
                }
            } else {
                Color.clear
            }
        }
        .frame(width: QuickLookMetrics.width, height: QuickLookMetrics.height)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: QuickLookMetrics.radius))
        .overlay(
            RoundedRectangle(cornerRadius: QuickLookMetrics.radius)
                .strokeBorder(.separator, lineWidth: 1))
    }

    private func header(_ view: QuickLookViewFfi) -> some View {
        HStack(spacing: 12) {
            Text(view.title).font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
            Text(QuickLookWords.header(view)).font(.system(size: 12)).foregroundStyle(.tertiary)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            ForEach(Array(view.actions.enumerated()), id: \.offset) { at, action in
                Button {
                    switch at {
                    case 0: model.open()
                    case 1: model.archive()
                    default: model.close()
                    }
                } label: {
                    HStack(spacing: 6) {
                        Text(action.label).font(.system(size: 12.5, weight: .semibold)).foregroundStyle(.primary)
                        KeyCap(KeyCapSpelling.cap(action.key) ?? action.key)
                    }
                    .padding(.leading, 10)
                    .padding(.trailing, 8)
                    .frame(height: 28)
                    .background(
                        RoundedRectangle(cornerRadius: 6).fill(at == 0 ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear)))
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .lineLimit(1)
        .padding(.horizontal, 14)
        .frame(height: QuickLookMetrics.header)
        .background(QuickLookMetrics.tint)
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
    }

    private func content(_ view: QuickLookViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 4) {
                Text(SearchRuns.attributed(view.subject, size: 22, weight: .bold, scheme: scheme))
                    .lineSpacing(28 - 22 * 1.2)
                    .lineLimit(2)
                Text(sender(view.sender)).lineLimit(1).truncationMode(.tail)
            }
            HStack(spacing: 8) {
                Text(view.matchesLine).font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
                if let hint = QuickLookWords.matchesHint(view) {
                    Text(hint).font(.system(size: 12)).foregroundStyle(.tertiary)
                }
            }
            .lineLimit(1)
            .padding(.top, 16)
            .padding(.bottom, 10)
            ScrollViewReader { scroller in
                ScrollView(.vertical) {
                    VStack(spacing: 8) {
                        ForEach(Array(view.cards.enumerated()), id: \.offset) { at, card in
                            MatchCardView(card: card, current: view.current == UInt32(at))
                                .id(at)
                        }
                    }
                    .padding(.bottom, 22)
                }
                .scrollIndicators(.never)
                .onChange(of: view.current) { _, current in
                    guard let current else { return }
                    scroller.scrollTo(Int(current))
                }
            }
        }
        .padding(.top, 22)
        .padding(.horizontal, 40)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    /// The sender line at 13: the name strong, the address in SF Mono 12
    /// tertiary, the rest secondary.
    private func sender(_ runs: [RunFfi]) -> AttributedString {
        var text = AttributedString()
        for run in runs {
            var piece = AttributedString(run.text)
            switch run.style {
            case .strong:
                piece.font = .system(size: 13, weight: .semibold)
                piece.foregroundColor = .primary
            case .mono:
                piece.font = .system(size: 12, design: .monospaced)
                piece.foregroundColor = Color(nsColor: .tertiaryLabelColor)
            case .plain:
                piece.font = .system(size: 13)
                piece.foregroundColor = .secondary
            }
            text += piece
        }
        return text
    }
}

/// One match (§3.7): where and when in the 120 column, the passage at
/// 14/22; the current one on the accent's tint with its 2-point ring.
struct MatchCardView: View {
    let card: MatchCardFfi
    let current: Bool
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text(card.place).font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary)
                    .lineLimit(3)
                if !card.when.isEmpty {
                    Text(card.when).font(.system(size: 12)).foregroundStyle(.tertiary).lineLimit(2)
                }
            }
            .frame(width: QuickLookMetrics.placeColumn, alignment: .leading)
            Text(SearchRuns.attributed(card.passage, size: 14, scheme: scheme))
                .lineSpacing(22 - 14 * 1.2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 12)
        .padding(.horizontal, 14)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(current ? AnyShapeStyle(Color.accentColor.opacity(SearchRuns.focusFill(scheme))) : AnyShapeStyle(QuickLookMetrics.tint)))
        .overlay {
            if current {
                RoundedRectangle(cornerRadius: 8).strokeBorder(Color.accentColor, lineWidth: 2)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(current ? .isSelected : [])
    }
}
