import Foundation
import Observation
import PostioFFI

public protocol CommandBarEngine: AnyObject {
    func focusBarTyped(_ text: String)
    func focusBarRun(_ token: UInt64)
    func focusBarTab() -> Bool
    func invoke(_ id: String)
    func binding(for command: String) -> String?
    func focusSearchHighlighted(_ token: UInt64)
    func focusSearchForget(_ token: UInt64)
    func focusSearchShowAll()
    func focusSearchExclude(_ token: UInt64)
}

extension PostioSession: CommandBarEngine {}

/// Not `Intercepted`: every one goes to `invoke`, and the controller answers it.
public enum BarCommand {
    public static let search = "search"
    public static let palette = "command_palette"
    public static let back = "back"
    public static let saveSearch = "save_search"
    public static let folders = "go_to_folders"
}

/// The one thing decided here is the arrows' highlight, which the controller leaves to the toolkit:
/// `FocusBarLines` names a
/// line only when a run moved it there. No AppKit, so iOS stays reachable (#1264).
@MainActor
@Observable
public final class CommandBarModel {
    public enum Change: Equatable {
        /// `selection` is in UTF-16 units, as the field counts.
        case open(text: String, selection: NSRange)
        case lines
        case close
    }

    public struct Row: Equatable, Identifiable {
        public let id: UInt64
        public let kind: BarLineKindFfi
        public let title: String
        public let detail: String?
        public let cap: String?
        public let selectable: Bool
    }

    public struct Saved: Equatable {
        public let name: String
        public let cap: String?
    }

    public static let savedCommands = (1...4).map { "saved_search_\($0)" }

    public private(set) var isOpen = false
    public private(set) var mode: BarModeFfi = .search
    public private(set) var text = ""
    public private(set) var rows: [Row] = []
    public private(set) var echo: String?
    public private(set) var saved: [Saved] = []
    public private(set) var highlighted: UInt64?
    public let dropdown: DropdownModel
    public private(set) var showsDropdown = false

    @ObservationIgnored private let engine: CommandBarEngine

    public init(engine: CommandBarEngine) {
        self.engine = engine
        dropdown = DropdownModel(engine: engine)
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusOpenBar(mode, text, select):
            isOpen = true
            self.mode = mode
            self.text = text
            let selection = select.map { Self.range(of: $0, in: text) }
                ?? NSRange(location: (text as NSString).length, length: 0)
            return .open(text: text, selection: selection)
        case let .focusBarLines(view):
            guard isOpen else { return nil }
            if showsDropdown {
                showsDropdown = false
                dropdown.forget()
            }
            draw(view)
            return .lines
        case let .focusDropdown(view):
            guard isOpen else { return nil }
            showsDropdown = true
            dropdown.draw(view)
            return .lines
        case .focusCloseSurface(kind: .bar):
            guard isOpen else { return nil }
            forget()
            return .close
        default:
            return nil
        }
    }

    public func typed(_ text: String) {
        guard isOpen, text != self.text else { return }
        self.text = text
        engine.focusBarTyped(text)
    }

    public func move(by delta: Int) {
        if showsDropdown { return dropdown.move(by: delta) }
        let selectable = rows.filter(\.selectable).map(\.id)
        guard !selectable.isEmpty else { return }
        let at = highlighted.flatMap { selectable.firstIndex(of: $0) } ?? 0
        let next = min(max(at + delta, 0), selectable.count - 1)
        highlighted = selectable[next]
    }

    public func runHighlighted() {
        if isOpen, showsDropdown { return dropdown.runHighlighted() }
        guard isOpen, let highlighted else { return }
        engine.focusBarRun(highlighted)
    }

    public func run(_ token: UInt64) {
        guard isOpen, rows.contains(where: { $0.id == token && $0.selectable }) else { return }
        highlighted = token
        engine.focusBarRun(token)
    }

    @discardableResult
    public func showAll() -> Bool {
        guard isOpen, showsDropdown else { return false }
        dropdown.showAll()
        return true
    }

    public func forgetHighlighted() -> Bool {
        guard isOpen, showsDropdown else { return false }
        return dropdown.forgetHighlighted()
    }

    public func excludeHighlighted() -> Bool {
        guard isOpen, showsDropdown else { return false }
        return dropdown.excludeHighlighted()
    }

    public func back() {
        engine.invoke(BarCommand.back)
    }

    public func tab() -> Bool {
        guard isOpen else { return false }
        return engine.focusBarTab()
    }

    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        forget()
        return true
    }

    private func draw(_ view: BarViewFfi) {
        rows = view.lines.map { line in
            Row(
                id: line.token, kind: line.kind, title: line.title, detail: line.detail,
                cap: cap(for: line), selectable: line.selectable)
        }
        echo = view.echo
        saved = view.saved.enumerated().map { index, name in
            let command = index < Self.savedCommands.count ? Self.savedCommands[index] : nil
            return Saved(name: name, cap: command.flatMap { KeyCapSpelling.cap(engine.binding(for: $0)) })
        }
        let runs = rows.filter(\.selectable).map(\.id)
        if let named = view.highlight, runs.contains(named) {
            highlighted = named
        } else if let kept = highlighted, runs.contains(kept) {
        } else {
            highlighted = runs.first
        }
    }

    private func cap(for line: BarLineFfi) -> String? {
        if let command = line.command {
            return KeyCapSpelling.cap(engine.binding(for: command) ?? line.key)
        }
        return KeyCapSpelling.cap(line.key)
    }

    private func forget() {
        isOpen = false
        text = ""
        rows = []
        echo = nil
        saved = []
        highlighted = nil
        showsDropdown = false
        dropdown.forget()
    }

    /// The controller counts Unicode scalars (Rust `char`); this is the UTF-16 range the field
    /// selects.
    static func range(of select: BarSelectFfi, in text: String) -> NSRange {
        let scalars = text.unicodeScalars
        func index(_ count: UInt32) -> String.Index {
            scalars.index(scalars.startIndex, offsetBy: Int(count), limitedBy: scalars.endIndex)
                ?? scalars.endIndex
        }
        let start = index(min(select.start, select.end))
        let end = index(max(select.start, select.end))
        return NSRange(start..<end, in: text)
    }
}

extension CommandBarModel {
    public func runSaved(_ index: Int) {
        guard isOpen, saved.indices.contains(index), index < Self.savedCommands.count else { return }
        engine.invoke(Self.savedCommands[index])
    }
}

public enum CommandBarGeometry {
    public static let searchWidth: CGFloat = 860
    public static let restingWidth: CGFloat = 320
    public static let edge: CGFloat = 12
    public static let leading: CGFloat = 168
    public static let gap: CGFloat = 6
    public static let margin: CGFloat = 8

    public static func fieldWidth(window: CGFloat) -> CGFloat {
        max(min(searchWidth, window - edge - leading), restingWidth)
    }

    public static func frame(field: CGRect, window: CGRect, height: CGFloat) -> CGRect {
        let top = field.minY - gap
        let tallest = max(top - (window.minY + margin), 0)
        let tall = min(height, tallest)
        let x = max(field.minX, window.minX)
        let width = min(field.width, window.maxX - x)
        return CGRect(x: x, y: top - tall, width: width, height: tall)
    }
}
