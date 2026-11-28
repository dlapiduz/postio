import Observation
import PostioFFI
import SwiftUI

public protocol NoResultsEngine: AnyObject {
    func invoke(_ id: String)
}

extension PostioSession: NoResultsEngine {}

/// The controller decides everything; a click on a way out runs it as its number key would. No
/// AppKit (#1264).
@MainActor
@Observable
public final class NoResultsModel {
    public enum Change: Equatable {
        case open
        case redraw
        case close
    }

    public private(set) var view: NoResultsViewFfi?

    @ObservationIgnored private let engine: NoResultsEngine

    public init(engine: NoResultsEngine) {
        self.engine = engine
    }

    public var isOpen: Bool { view != nil }

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

    public func pick(_ number: UInt32) {
        guard let view, view.relaxations.contains(where: { $0.number == number }) else { return }
        engine.invoke(NoResultsView.command(number))
    }
}

public struct NoResultsView: View {
    @Environment(\.colorScheme) private var scheme
    let model: NoResultsModel

    public init(model: NoResultsModel) {
        self.model = model
    }

    public static let width: CGFloat = 560
    public static let top: CGFloat = 70

    public static func command(_ number: UInt32) -> String { "pick_relaxation_\(number)" }

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
                        // One line so every way out is 50 tall; a long query tightens, then gives
                        // up its middle, and the whole query is the tooltip and what a screen
                        // reader hears.
                        .lineLimit(1)
                        .minimumScaleFactor(0.85)
                        .allowsTightening(true)
                        .truncationMode(.middle)
                        .help(way.query)
                }
                .layoutPriority(1)
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
            .background(way.focused ? AnyShapeStyle(Color.accentColor.opacity(SearchRuns.focusFill(scheme))) : AnyShapeStyle(.clear))
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
