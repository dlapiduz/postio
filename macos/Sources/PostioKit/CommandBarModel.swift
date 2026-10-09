import Foundation
import Observation
import PostioFFI

/// What the command bar tells the engine, and asks it (specs/009-focus-macos
/// T084).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `CommandBarModel` without a store. Every call is cheap: the
/// controller answers in memory and says what changed as `UiEvent`s.
public protocol CommandBarEngine: AnyObject {
    /// The field holds `text` now.
    func focusBarTyped(_ text: String)
    /// Run the line with `token`: Return on the highlighted line, or a click.
    func focusBarRun(_ token: UInt64)
    /// `Tab` in the field: `false` when there is no chip to step into, and
    /// the key is the toolkit's.
    func focusBarTab() -> Bool
    /// A registry command, as a key press would send it: Back, for Escape.
    func invoke(_ id: String)
    /// The binding in force for `command`, for its keycap.
    func binding(for command: String) -> String?
    /// The search dropdown's arrows rest on the row `token` now
    /// (specs/010-focus-search): what ⌥⌫ from a menu forgets.
    func focusSearchHighlighted(_ token: UInt64)
    /// ⌥⌫ on the dropdown's recent search `token`.
    func focusSearchForget(_ token: UInt64)
    /// ⌘↩ in the dropdown: every result for what is typed.
    func focusSearchShowAll()
    /// ⌥↩ on the dropdown's person, label or folder `token`: its chip,
    /// excluded (specs/010-focus-search US7).
    func focusSearchExclude(_ token: UInt64)
}

extension PostioSession: CommandBarEngine {}

/// The registry commands the bar and its field run by name. Not
/// `Intercepted`: every one goes to `invoke`, and Focus's controller
/// answers it (`CommandBarModelTests` checks each is a real command).
public enum BarCommand {
    /// `/`: the bar on search. A click into the toolbar's field runs it too.
    public static let search = "search"
    /// ⌘K: the bar on commands, `>` typed.
    public static let palette = "command_palette"
    /// Escape: the bar closes through the controller's Back.
    public static let back = "back"
    /// ⌘S: the current query, saved.
    public static let saveSearch = "save_search"
    /// `g o`: the folders popover.
    public static let folders = "go_to_folders"
}

/// The command bar's state, as the controller's intents leave it
/// (specs/009-focus-macos T084, T085; screens 07 to 09).
///
/// The bar is Focus's controller's (`crates/postio-focus/src/bar.rs`): what
/// typing means, which lines it offers, what each says and what running one
/// does. This holds what the last `FocusOpenBar` and `FocusBarLines` said,
/// spelled for drawing -- each line one row, with the keycap of the user's
/// binding -- and hands back what the toolkit saw: the field's words, the
/// line Return or a click ran, `Tab`, and Escape (as the controller's Back).
///
/// The one thing decided here is the arrows' highlight, which the
/// controller leaves to the toolkit: `FocusBarLines` names a line only when
/// a run moved it there.
///
/// No AppKit: the panel (`CommandBarPanel`) and the SwiftUI view
/// (`CommandBarView`) both read it, and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class CommandBarModel {
    /// What one applied event changed, for the panel and the field.
    public enum Change: Equatable {
        /// Show the bar (or keep it shown) with `text` in the field and
        /// `selection` selected: the chip being edited, or the caret at the
        /// end. In UTF-16 units, as the field counts.
        case open(text: String, selection: NSRange)
        /// The lines, the chips or the saved row changed.
        case lines
        /// The bar is gone.
        case close
    }

    /// One line of the bar, as drawn.
    public struct Row: Equatable, Identifiable {
        /// The line's token: what running it hands back.
        public let id: UInt64
        public let kind: BarLineKindFfi
        public let title: String
        public let detail: String?
        /// The keycap: the user's binding for the line's command, or the
        /// key the line names (a hint's `>`).
        public let cap: String?
        /// Whether the arrows may rest on it and Return run it.
        public let selectable: Bool
        public let sender: String?
        public let wheres: [String]
        public let time: String?
    }

    /// One pinned saved search, and the key that runs it (`⌥1`-`⌥4`).
    public struct Saved: Equatable {
        public let name: String
        public let cap: String?
    }

    /// The commands the saved row's keys run, in order.
    public static let savedCommands = (1...4).map { "saved_search_\($0)" }

    /// Whether the bar is up.
    public private(set) var isOpen = false
    /// How it was opened: `/` or ⌘K.
    public private(set) var mode: BarModeFfi = .search
    /// The field's words, as the controller last set them or the person
    /// last typed them.
    public private(set) var text = ""
    public private(set) var rows: [Row] = []
    /// The chips the words were lowered to, drawn before the lines.
    public private(set) var chips: [String] = []
    /// The chip being edited after `Tab`.
    public private(set) var editing: Int?
    /// The line under the field: what was typed, or which chip is edited.
    public private(set) var echo: String?
    /// The heading over a folder's conversations (`in:`).
    public private(set) var heading: String?
    public private(set) var saved: [Saved] = []
    /// The highlighted line's token.
    public private(set) var highlighted: UInt64?
    /// The search dropdown (specs/010-focus-search), drawn in place of the
    /// lines while `showsDropdown`: the controller sends one or the other.
    public let dropdown: DropdownModel
    /// Whether the last view was the dropdown's.
    public private(set) var showsDropdown = false

    @ObservationIgnored private let engine: CommandBarEngine

    public init(engine: CommandBarEngine) {
        self.engine = engine
        dropdown = DropdownModel(engine: engine)
    }

    /// Apply `event` if it is the bar's, and say what it changed; `nil` for
    /// every other event.
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

    /// The field's words changed. Said to the controller unless they are
    /// what it put there itself.
    public func typed(_ text: String) {
        guard isOpen, text != self.text else { return }
        self.text = text
        engine.focusBarTyped(text)
    }

    /// ↑ or ↓: the next line that runs, stopping at either end.
    public func move(by delta: Int) {
        if showsDropdown { return dropdown.move(by: delta) }
        let selectable = rows.filter(\.selectable).map(\.id)
        guard !selectable.isEmpty else { return }
        let at = highlighted.flatMap { selectable.firstIndex(of: $0) } ?? 0
        let next = min(max(at + delta, 0), selectable.count - 1)
        highlighted = selectable[next]
    }

    /// Return: run the highlighted line.
    public func runHighlighted() {
        if isOpen, showsDropdown { return dropdown.runHighlighted() }
        guard isOpen, let highlighted else { return }
        engine.focusBarRun(highlighted)
    }

    /// A click on the line with `token`.
    public func run(_ token: UInt64) {
        guard isOpen, rows.contains(where: { $0.id == token && $0.selectable }) else { return }
        highlighted = token
        engine.focusBarRun(token)
    }

    /// ⌘↩: the dropdown's Show all. `false` when the lines are up, and
    /// the key is not the bar's.
    @discardableResult
    public func showAll() -> Bool {
        guard isOpen, showsDropdown else { return false }
        dropdown.showAll()
        return true
    }

    /// ⌥⌫: forget the dropdown's highlighted recent search. `false` leaves
    /// the key to the field.
    public func forgetHighlighted() -> Bool {
        guard isOpen, showsDropdown else { return false }
        return dropdown.forgetHighlighted()
    }

    /// ⌥↩: exclude the dropdown's highlighted person, label or folder.
    /// `false` leaves the key to the field.
    public func excludeHighlighted() -> Bool {
        guard isOpen, showsDropdown else { return false }
        return dropdown.excludeHighlighted()
    }

    /// Escape: the controller's Back, which closes the bar and says so.
    public func back() {
        engine.invoke(BarCommand.back)
    }

    /// `Tab`: whether the bar used it. `false` leaves it to the toolkit.
    public func tab() -> Bool {
        guard isOpen else { return false }
        return engine.focusBarTab()
    }

    /// The toolkit closed the bar -- a click outside, the field giving up
    /// the keyboard. `true` when it was open, and the controller must be
    /// told (`focusSurfaceClosed(.bar)`).
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        forget()
        return true
    }

    // MARK: -

    private func draw(_ view: BarViewFfi) {
        rows = view.lines.map { line in
            Row(
                id: line.token, kind: line.kind, title: line.title, detail: line.detail,
                cap: cap(for: line), selectable: line.selectable, sender: line.sender,
                wheres: line.wheres, time: line.time)
        }
        chips = view.chips
        editing = view.editing.map(Int.init)
        echo = view.echo
        heading = view.heading
        saved = view.saved.enumerated().map { index, name in
            let command = index < Self.savedCommands.count ? Self.savedCommands[index] : nil
            return Saved(name: name, cap: command.flatMap { KeyCapSpelling.cap(engine.binding(for: $0)) })
        }
        let runs = rows.filter(\.selectable).map(\.id)
        if let named = view.highlight, runs.contains(named) {
            highlighted = named
        } else if let kept = highlighted, runs.contains(kept) {
            // Results landing under the same words: the arrows' place stays.
        } else {
            highlighted = runs.first
        }
    }

    /// The keycap a line draws: the binding in force for its command, as
    /// every keycap on the Mac is spelled, or the key it names itself.
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
        chips = []
        editing = nil
        echo = nil
        heading = nil
        saved = []
        highlighted = nil
        showsDropdown = false
        dropdown.forget()
    }

    /// `select`, which counts characters as the controller does (Rust's
    /// `char`, a Unicode scalar), as the UTF-16 range the field selects.
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
    /// A click on the saved row's `index`th search: what its key runs
    /// (`saved_search_<n>`), so the click and `⌥n` cannot disagree.
    public func runSaved(_ index: Int) {
        guard isOpen, saved.indices.contains(index), index < Self.savedCommands.count else { return }
        engine.invoke(Self.savedCommands[index])
    }
}

/// Where the search field grows and the panel hangs (specs/010-focus-search
/// T054; design §2 "Opening and layout"): while the bar is up the toolbar's
/// field grows leftward to 860 wide, its right edge 12 from the window's,
/// giving way to the toolbar's leading items in a narrow window; the panel
/// hangs 6 below the field with its left edge and width, and inside its
/// window. Screen coordinates, the origin at the bottom left, as AppKit has
/// them.
public enum CommandBarGeometry {
    /// The field's width while the bar is up.
    public static let searchWidth: CGFloat = 860
    /// The field's width while it is not.
    public static let restingWidth: CGFloat = 320
    /// Between the field's right edge and the window's.
    public static let edge: CGFloat = 12
    /// What the toolbar keeps on the left of a grown field: the window's
    /// buttons and Compose.
    public static let leading: CGFloat = 160
    /// Between the field's bottom and the panel's top.
    public static let gap: CGFloat = 6
    /// The least the panel keeps from its window's bottom.
    public static let margin: CGFloat = 8

    /// The field's width while the bar is up, in a window `window` wide.
    public static func fieldWidth(window: CGFloat) -> CGFloat {
        max(min(searchWidth, window - edge - leading), restingWidth)
    }

    /// The panel's frame under a field at `field` in a window at `window`,
    /// wanting `height`.
    public static func frame(field: CGRect, window: CGRect, height: CGFloat) -> CGRect {
        let top = field.minY - gap
        let tallest = max(top - (window.minY + margin), 0)
        let tall = min(height, tallest)
        let x = max(field.minX, window.minX)
        let width = min(field.width, window.maxX - x)
        return CGRect(x: x, y: top - tall, width: width, height: tall)
    }
}
