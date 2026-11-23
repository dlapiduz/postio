import Observation
import PostioFFI
import SwiftUI

// The controller composes it (`postio_ui::thread_outline::menu`) and decides what each entry does;
// this draws it and says which was chosen.

public enum ThreadMenuCommand {
    public static let menu = "thread_menu"
}

public protocol ThreadMenuEngine: AnyObject {
    func focusThreadMenuPick(command: String)
}

extension PostioSession: ThreadMenuEngine {}

@MainActor
@Observable
public final class ThreadMenuModel {
    public enum Change: Equatable {
        case open
        case close
    }

    public private(set) var view: ThreadMenuFfi?
    /// `nil` before the first arrow, where Return does nothing.
    public private(set) var highlighted: String?

    @ObservationIgnored private let engine: ThreadMenuEngine

    public init(engine: ThreadMenuEngine) {
        self.engine = engine
    }

    public var isOpen: Bool { view != nil }

    public var pickable: [ThreadMenuEntryFfi] {
        (view?.groups ?? []).flatMap(\.entries).filter(\.enabled)
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusThreadMenu(next):
            view = next
            highlighted = nil
            return .open
        case .focusCloseSurface(.threadMenu):
            guard view != nil else { return nil }
            view = nil
            highlighted = nil
            return .close
        default:
            return nil
        }
    }

    public func move(by step: Int) {
        let commands = pickable.map(\.command)
        guard !commands.isEmpty else { return }
        guard let current = highlighted.flatMap(commands.firstIndex(of:)) else {
            highlighted = step > 0 ? commands.first : commands.last
            return
        }
        highlighted = commands[min(max(current + step, 0), commands.count - 1)]
    }

    public func confirm() {
        guard let highlighted else { return }
        pick(highlighted)
    }

    public func pick(_ command: String) {
        guard pickable.contains(where: { $0.command == command }) else { return }
        engine.focusThreadMenuPick(command: command)
    }

    /// The controller is told by whoever presents it. `true` when it was up.
    @discardableResult
    public func closedByToolkit() -> Bool {
        defer { view = nil; highlighted = nil }
        return view != nil
    }
}

public enum ThreadMenuMetrics {
    public static let width: CGFloat = 340
}

func threadKeycap(_ binding: String?) -> String? {
    guard let binding else { return nil }
    let caps = binding.components(separatedBy: ", ").map { KeyCapSpelling.cap($0) ?? $0 }
    return caps.joined(separator: ", ")
}

public struct ThreadMenuView: View {
    let model: ThreadMenuModel

    public init(model: ThreadMenuModel) {
        self.model = model
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            let groups = model.view?.groups ?? []
            ForEach(Array(groups.enumerated()), id: \.offset) { offset, group in
                if offset > 0 { Divider().padding(.vertical, 6).padding(.horizontal, 12) }
                Text(group.title)
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 16)
                    .padding(.top, offset == 0 ? 10 : 2)
                    .padding(.bottom, 2)
                    .accessibilityAddTraits(.isHeader)
                ForEach(group.entries, id: \.command) { entry in
                    ThreadMenuRow(entry: entry, highlighted: model.highlighted == entry.command) {
                        model.pick(entry.command)
                    }
                }
            }
            Color.clear.frame(height: 6)
        }
        .frame(width: ThreadMenuMetrics.width)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Thread")
    }
}

struct ThreadMenuRow: View {
    let entry: ThreadMenuEntryFfi
    let highlighted: Bool
    let pick: () -> Void

    var body: some View {
        Button(action: pick) {
            HStack(alignment: .center, spacing: 8) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(entry.title).font(.system(size: 13))
                    if let note = entry.note {
                        Text(note)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 8)
                if let cap = threadKeycap(entry.keycap) { KeyCap(cap) }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(highlighted ? Color.primary.opacity(0.08) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 6)
        .foregroundStyle(entry.enabled ? .primary : .tertiary)
        .disabled(!entry.enabled)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel([entry.title, entry.keycap.flatMap(threadKeycap), entry.note]
            .compactMap { $0 }.joined(separator: ", "))
        .accessibilityAddTraits(highlighted ? .isSelected : [])
    }
}
