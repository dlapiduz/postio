import Observation
import PostioFFI
import SwiftUI

/// What the no-results page tells the engine (specs/010-focus-search
/// T106).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `NoResultsModel` without a store.
public protocol NoResultsEngine: AnyObject {
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
}

extension PostioSession: NoResultsEngine {}

/// The page a search that found nothing shows in the rows' place, as the
/// controller's last `FocusRelaxations` drew it (design §3.10, screen 13).
///
/// The controller decides everything: the title, the ways out and their
/// order, which one is focused, when the counts have landed, and when the
/// page goes (a new query, a way out run, `None`). A click on a way out
/// runs it exactly as its number key would. Leaving the results takes the
/// page with them.
///
/// No AppKit (#1264): `NoResultsView` draws it.
@MainActor
@Observable
public final class NoResultsModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// The page is up, in the rows' place.
        case open
        /// The page was drawn again: the counts landed, the focus moved.
        case redraw
        /// The page is gone.
        case close
    }

    /// What it draws; `nil` while the results found something.
    public private(set) var view: NoResultsViewFfi?

    @ObservationIgnored private let engine: NoResultsEngine

    public init(engine: NoResultsEngine) {
        self.engine = engine
    }

    /// Whether the page is up.
    public var isOpen: Bool { view != nil }

    /// Apply `event` if it is the page's, and say what it changed; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusRelaxations(next):
            guard let next else {
                guard view != nil else { return nil }
                view = nil
                return .close
            }
            let was = view != nil
            view = next
            return was ? .redraw : .open
        case .focusLeaveResults:
            guard view != nil else { return nil }
            view = nil
            return .close
        default:
            return nil
        }
    }

    /// A click on way out `number` (from 1): run, as its key runs it.
    public func pick(_ number: UInt32) {
        guard let view, view.relaxations.contains(where: { $0.number == number }) else { return }
        engine.invoke(NoResultsView.command(number))
    }
}

/// The no-results page (design §3.10, screen 13): centred, 560 wide, 70
/// below the filter bar. "Nothing matches all four filters" 20 bold; the
/// sentence 14 secondary; the ways out in a rounded box, each row 50 tall
/// with its number's cap, what it changes over the query in SF Mono, and
/// its count bold on the right, the focused one on the accent's tint with
/// its ring; then how much was searched, 12.5 tertiary. Every word is the
/// controller's (`NoResultsViewFfi`).
public struct NoResultsView: View {
    let model: NoResultsModel

    public init(model: NoResultsModel) {
        self.model = model
    }

    public static let width: CGFloat = 560
    public static let top: CGFloat = 70

    /// The registry command a way out's number runs.
    public static func command(_ number: UInt32) -> String { "pick_relaxation_\(number)" }

    /// A way out's cap: its key as the keymap spells it, else its number.
    public static func cap(for way: RelaxationFfi) -> String {
        KeyCapSpelling.cap(way.key) ?? String(way.number)
    }

    public var body: some View {
        if let view = model.view {
            VStack(alignment: .leading, spacing: 14) {
                Text(view.title)
                    .font(.system(size: 20, weight: .bold))
                    .accessibilityAddTraits(.isHeader)
                Text(view.body)
                    .font(.system(size: 14))
                    .lineSpacing(14 * 0.55 - 4)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if !view.relaxations.isEmpty || view.counting != nil {
                    ways(view)
                }
                Text(view.searched)
                    .font(.system(size: 12.5))
                    .lineSpacing(12.5 * 0.55 - 3)
                    .foregroundStyle(.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(width: Self.width, alignment: .leading)
            .padding(.top, Self.top)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .accessibilityElement(children: .contain)
        }
    }

    private func ways(_ view: NoResultsViewFfi) -> some View {
        VStack(spacing: 0) {
            ForEach(Array(view.relaxations.enumerated()), id: \.element.number) { at, way in
                row(way)
                if at + 1 < view.relaxations.count || view.counting != nil {
                    Divider()
                }
            }
            if let counting = view.counting {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(counting).font(.system(size: 13)).foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 14)
                .frame(minHeight: 50)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(.separator, lineWidth: 1))
    }

    private func row(_ way: RelaxationFfi) -> some View {
        Button {
            model.pick(way.number)
        } label: {
            HStack(spacing: 12) {
                KeyCap(Self.cap(for: way)).frame(width: 24)
                VStack(alignment: .leading, spacing: 2) {
                    Text(way.label).font(.system(size: 13.5)).foregroundStyle(.primary)
                    Text(way.query)
                        .font(.system(size: 11.5, design: .monospaced))
                        .foregroundStyle(.tertiary)
                        // A query with whole addresses outgrows one line
                        // beside its count; it wraps rather than hide a
                        // term in an ellipsis.
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                Text(way.count)
                    .font(.system(size: 13, weight: .bold))
                    .foregroundStyle(.primary)
                    .fixedSize()
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .frame(minHeight: 50)
            .contentShape(Rectangle())
            .background(way.focused ? AnyShapeStyle(Color.accentColor.opacity(0.08)) : AnyShapeStyle(.clear))
            .overlay {
                if way.focused {
                    RoundedRectangle(cornerRadius: 10).strokeBorder(Color.accentColor, lineWidth: 2)
                }
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(way.number), \(way.label), \(way.query), \(way.count)")
    }
}
