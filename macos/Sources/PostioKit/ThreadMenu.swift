import Observation
import PostioFFI
import SwiftUI

// Thread ▾'s menu (spec 013 US5, T066; the threads SPEC section 4, screen
// 04): a 340-point popover from the button in the thread bar, two groups of
// entries, each with its keycap, an optional one-line note, and dimmed when
// it does nothing here.
//
// The controller composes it (`postio_ui::thread_outline::menu`) and decides
// what each entry does; this draws it and says which entry was chosen.

/// The commands the thread bar runs.
public enum ThreadMenuCommand {
    /// Open Thread ▾'s menu: `alt+period`.
    public static let menu = "thread_menu"
}

/// What the menu tells the engine. `PostioSession` is the one real answer;
/// the protocol is what lets a test drive `ThreadMenuModel` without a store.
public protocol ThreadMenuEngine: AnyObject {
    /// An entry was chosen by click or Return: its command's id.
    func focusThreadMenuPick(command: String)
}

extension PostioSession: ThreadMenuEngine {}

/// The menu, as the controller's last `FocusThreadMenu` drew it.
@MainActor
@Observable
public final class ThreadMenuModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// It opened: hang it from Thread ▾.
        case open
        /// The controller put it away.
        case close
    }

    /// What it draws; `nil` while it is closed.
    public private(set) var view: ThreadMenuFfi?
    /// The entry the arrows are on, as a command id; `nil` before the first
    /// arrow, where Return does nothing.
    public private(set) var highlighted: String?

    @ObservationIgnored private let engine: ThreadMenuEngine

    public init(engine: ThreadMenuEngine) {
        self.engine = engine
    }

    /// Whether it is up.
    public var isOpen: Bool { view != nil }

    /// The entries that can be chosen, top to bottom across both groups.
    public var pickable: [ThreadMenuEntryFfi] {
        (view?.groups ?? []).flatMap(\.entries).filter(\.enabled)
    }

    /// Apply `event` if it is the menu's, and say what it changed.
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

    /// ↑ or ↓: the highlight steps through the entries that can be chosen,
    /// and stops at the ends.
    public func move(by step: Int) {
        let commands = pickable.map(\.command)
        guard !commands.isEmpty else { return }
        guard let current = highlighted.flatMap(commands.firstIndex(of:)) else {
            highlighted = step > 0 ? commands.first : commands.last
            return
        }
        highlighted = commands[min(max(current + step, 0), commands.count - 1)]
    }

    /// Return: choose the highlighted entry.
    public func confirm() {
        guard let highlighted else { return }
        pick(highlighted)
    }

    /// Choose the entry running `command`; a dimmed one does nothing.
    public func pick(_ command: String) {
        guard pickable.contains(where: { $0.command == command }) else { return }
        engine.focusThreadMenuPick(command: command)
    }

    /// The toolkit took it down: a click away. The controller is told by
    /// whoever presents it. `true` when it was up.
    @discardableResult
    public func closedByToolkit() -> Bool {
        defer { view = nil; highlighted = nil }
        return view != nil
    }
}

/// The menu's measures, in points.
public enum ThreadMenuMetrics {
    public static let width: CGFloat = 340
}

/// A keymap spelling as keycaps' text: "alt+t" is ⌥T, "f, 2" is two caps'
/// worth, kept as one line.
func threadKeycap(_ binding: String?) -> String? {
    guard let binding else { return nil }
    let caps = binding.components(separatedBy: ", ").map { KeyCapSpelling.cap($0) ?? $0 }
    return caps.joined(separator: ", ")
}

/// The menu: titled groups, a hairline between them.
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

/// One entry: its title and keycap, and its note under them.
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
