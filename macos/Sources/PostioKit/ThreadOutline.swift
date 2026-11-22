import Observation
import PostioFFI
import SwiftUI

// The controller keeps the filter, person and cursor and hands over the whole outline on every
// change (`FocusOutline`); this draws it and reports what the person did.

public protocol ThreadOutlineEngine: AnyObject {
    func focusOutlineTyped(text: String)
    func focusOutlinePerson(name: String?)
    func focusOutlineMove(by: Int32)
    func focusOutlineShow(row: UInt32?)
    func focusOutlineFocusFilter()
}

extension PostioSession: ThreadOutlineEngine {}

@MainActor
@Observable
public final class ThreadOutlineModel {
    public enum Change: Equatable {
        case open
        case redraw
        case close
        case focusFilter
    }

    public private(set) var view: OutlineFfi?

    @ObservationIgnored private let engine: ThreadOutlineEngine

    public init(engine: ThreadOutlineEngine) {
        self.engine = engine
    }

    public var isOpen: Bool { view != nil }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusOutline(next):
            let change: Change = view == nil ? .open : .redraw
            view = next
            return change
        case .focusOutlineFocusFilter:
            return view == nil ? nil : .focusFilter
        case .focusCloseSurface(.outline):
            guard view != nil else { return nil }
            view = nil
            return .close
        default:
            return nil
        }
    }

    public func typed(_ text: String) {
        guard let view, view.filter != text else { return }
        engine.focusOutlineTyped(text: text)
    }

    public func choose(person name: String) {
        guard let view else { return }
        engine.focusOutlinePerson(name: view.person == name ? nil : name)
    }

    public func move(by step: Int) {
        guard view != nil else { return }
        engine.focusOutlineMove(by: Int32(step))
    }

    public func show(row: Int? = nil) {
        guard view != nil else { return }
        engine.focusOutlineShow(row: row.map { UInt32($0) })
    }

    /// The controller is told by whoever presents it. `true` when it was up.
    @discardableResult
    public func closedByToolkit() -> Bool {
        defer { view = nil }
        return view != nil
    }
}

public enum ThreadOutlineMetrics {
    public static let width: CGFloat = 440
    public static let row: CGFloat = 34
    public static let field: CGFloat = 30
    public static let visibleRows: CGFloat = 14
}

public struct ThreadOutlineView<Field: View>: View {
    let model: ThreadOutlineModel
    let field: () -> Field

    /// `field` is AppKit's: `ThreadOutlinePopover` owns it because it must take the keyboard when
    /// the controller says.
    public init(model: ThreadOutlineModel, @ViewBuilder field: @escaping () -> Field) {
        self.model = model
        self.field = field
    }

    public var body: some View {
        let view = model.view
        VStack(alignment: .leading, spacing: 0) {
            field()
                .frame(height: ThreadOutlineMetrics.field)
                .padding(.horizontal, 12)
                .padding(.top, 12)
                .padding(.bottom, 8)
            people(view?.people ?? [], chosen: view?.person)
            Divider()
            rows(view)
            Divider()
            footer(view)
        }
        .frame(width: ThreadOutlineMetrics.width)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Thread outline")
    }

    private func people(_ people: [OutlinePersonFfi], chosen: String?) -> some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(people, id: \.name) { person in
                    let on = person.name == chosen
                    Button { model.choose(person: person.name) } label: {
                        HStack(spacing: 4) {
                            Text(person.name).font(.system(size: 11.5, weight: on ? .semibold : .regular))
                            Text("\(person.count)").font(.system(size: 11.5)).foregroundStyle(.secondary)
                        }
                        .padding(.horizontal, 8)
                        .frame(height: 22)
                        .background(Capsule().fill(.quaternary.opacity(0.7)))
                        .overlay(Capsule().strokeBorder(on ? Color.accentColor : .clear, lineWidth: 1.5))
                        .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("\(person.name), \(person.count) messages")
                    .accessibilityAddTraits(on ? .isSelected : [])
                }
            }
            .padding(.horizontal, 12)
        }
        .padding(.bottom, 10)
    }

    @ViewBuilder
    private func rows(_ view: OutlineFfi?) -> some View {
        let rows = view?.rows ?? []
        let cursor = Int(view?.cursor ?? 0)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(Array(rows.enumerated()), id: \.offset) { position, row in
                        OutlineRowView(row: row, cursor: position == cursor)
                            .id(position)
                            .onTapGesture { model.show(row: position) }
                    }
                }
            }
            .frame(height: min(CGFloat(max(rows.count, 1)), ThreadOutlineMetrics.visibleRows)
                * ThreadOutlineMetrics.row)
            .onChange(of: cursor, initial: true) { proxy.scrollTo($1, anchor: nil) }
        }
        if rows.isEmpty {
            Text("No message matches")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: ThreadOutlineMetrics.row)
        }
    }

    private func footer(_ view: OutlineFfi?) -> some View {
        HStack(spacing: 14) {
            hint("\u{2191}\u{2193}", "move")
            hint("\u{21a9}", "show it")
            if let cap = threadKeycap(view?.firstUnreadKey) { hint(cap, "first unread") }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private func hint(_ cap: String, _ label: String) -> some View {
        HStack(spacing: 5) {
            KeyCap(cap)
            Text(label).font(.system(size: 12)).foregroundStyle(.secondary)
        }
    }
}

struct OutlineRowView: View {
    let row: OutlineRowFfi
    let cursor: Bool

    var body: some View {
        HStack(spacing: 8) {
            Text("\(row.number)")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
                .frame(width: 22, alignment: .trailing)
            Text(row.initials)
                .font(.system(size: 8.5, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: 20, height: 20)
                .background(Circle().fill(.secondary))
            Text(row.name)
                .font(.system(size: 13, weight: row.bold ? .bold : .regular))
                .lineLimit(1)
                .frame(width: 84, alignment: .leading)
            Text(row.line)
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 4) {
                if row.unread { Circle().fill(Color.accentColor).frame(width: 6, height: 6) }
                Text(row.date).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: ThreadOutlineMetrics.row)
        .background(cursor ? Color.primary.opacity(0.07) : .clear)
        .overlay(Rectangle().strokeBorder(row.current ? Color.accentColor : .clear, lineWidth: 2))
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            ["Message \(row.number)", row.name, row.line, row.date, row.unread ? "unread" : nil]
                .compactMap { $0 }.joined(separator: ", "))
        .accessibilityAddTraits(row.current ? .isSelected : [])
    }
}
