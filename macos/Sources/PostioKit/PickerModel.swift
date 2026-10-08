import Foundation
import Observation
import PostioFFI

/// What a picker at the row tells the engine (specs/009-focus-macos T091).
///
/// `PostioSession` is the one real answer; the protocol is what lets a test
/// drive `PickerModel` without a store. Every call is cheap: the controller
/// answers in memory and says what changed as `UiEvent`s.
public protocol PickerEngine: AnyObject {
    /// The picker's field holds `text` now: the date typed, or the filter.
    func focusPickerTyped(_ text: String)
    /// The row `token` was chosen: a click, or Return on the highlight.
    func focusPickerChoose(_ token: UInt64)
    /// Space on the highlighted row `token`: a label on or off.
    func focusPickerToggle(_ token: UInt64)
    /// A registry command, as a key press would send it: Back, for Escape;
    /// the picker's confirm, for Return with nothing highlighted.
    func invoke(_ id: String)
    /// The binding in force for `command`, for its keycap.
    func binding(for command: String) -> String?
}

extension PostioSession: PickerEngine {}

/// The registry commands a picker's keys resolve to that only Swift can
/// answer, because they are about the highlighted row and only the toolkit
/// knows which that is. Every other picker key (`picker_choose_1`-`4`,
/// `picker_type_date`, `back`) goes to `invoke` as it resolved.
public enum PickerCommand {
    /// Space: the highlighted label on or off.
    public static let toggle = "picker_toggle"
    /// Return: the highlighted row chosen, or the controller's confirm.
    public static let confirm = "picker_confirm"
    /// Escape: the picker closes through the controller's Back.
    public static let back = "back"
}

/// A picker at the row -- snooze, remind, label or move -- as the
/// controller's intents leave it (specs/009-focus-macos T091, T092;
/// screens 11 to 14).
///
/// The pickers are Focus's controller's (`crates/postio-focus/src/pickers.rs`):
/// which opens, what it acts on, the rows it lists, what each says and what
/// choosing one does, what a typed date means. This holds what the last
/// `FocusOpenPicker` and `FocusPickerRows` said, spelled for drawing -- each
/// row's number key as a keycap, a label's dot in its colour -- and hands
/// back what the toolkit saw: the field's words, the row a click or Return
/// chose, the row Space toggled, and Escape as the controller's Back.
///
/// The one thing decided here is the highlight, which the controller leaves
/// to the toolkit: where the arrows put it, where it stands after a redraw,
/// and that typing a date takes it away (Return then means the date).
///
/// No AppKit: the popover (`PickerPopover`) and the SwiftUI view
/// (`PickerView`) both read it, and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class PickerModel {
    /// What one applied event changed, for the popover.
    public enum Change: Equatable {
        /// Show the picker hung from `anchor`, or show a new one in place.
        case open(anchor: PickerAnchorFfi)
        /// The rows, the field's words or its hint changed.
        case rows
        /// `Tab`: the date field takes the keyboard.
        case field
        /// The picker is gone.
        case close
    }

    /// One row of the picker, as drawn.
    public struct Row: Equatable, Identifiable {
        /// The row's token: what choosing or toggling it hands back.
        public let id: UInt64
        /// The heading drawn above it, where the controller starts a section.
        public let section: String?
        public let name: String
        /// On the right: a time, a count, "✓ applied".
        public let detail: String
        /// The keycap of the number key that chooses it.
        public let cap: String?
        /// Whether a label's dot is drawn before the name.
        public let dot: Bool
        /// The dot's colour; `nil` for the secondary colour (the boundary
        /// does not export `postio_ui::label_colour`, the places popover's
        /// gap too).
        public let colour: LabelColour?
        public let applied: Bool
        /// "Create label “…”".
        public let create: Bool
    }

    /// Whether a picker is up.
    public private(set) var isOpen = false
    public private(set) var kind: PickerKindFfi = .snooze
    /// What it hangs from: the cursor's row, or the message window.
    public private(set) var anchor: PickerAnchorFfi = .openMessage
    /// "Snooze until".
    public private(set) var title = ""
    /// What it acts on: "Ada Moreno · Atlas Q3 budget", "2 conversations".
    public private(set) var target = ""
    /// The date field under the rows, or the filter above them.
    public private(set) var field: PickerFieldFfi = .date
    public private(set) var placeholder = ""
    /// The field's words, as the controller last drew them or the person
    /// last typed them.
    public private(set) var typed = ""
    /// The date field's line: "Tab to type", or the date the words make.
    public private(set) var hint: String?
    public private(set) var rows: [Row] = []
    public private(set) var footnote = ""
    /// The highlighted row's token: what Space and Return are about.
    public private(set) var highlighted: UInt64?
    /// Whether the date field has the keyboard (after `Tab`).
    public private(set) var inField = false

    @ObservationIgnored private let engine: PickerEngine
    /// The words the rows on screen were drawn for: new words in a filter
    /// move the highlight to the first row they leave.
    @ObservationIgnored private var drawnFor = ""

    public init(engine: PickerEngine) {
        self.engine = engine
    }

    /// Whether the picker holds a field that takes the keyboard as it opens.
    public var filters: Bool { field == .filter }

    /// Apply `event` if it is a picker's, and say what it changed; `nil` for
    /// every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusOpenPicker(view):
            isOpen = true
            inField = false
            highlighted = nil
            draw(view, typedChanged: true)
            return .open(anchor: view.anchor)
        case let .focusPickerRows(view):
            guard isOpen else { return nil }
            draw(view, typedChanged: view.typed != drawnFor)
            return .rows
        case .focusPickerField:
            guard isOpen, field == .date else { return nil }
            inField = true
            // Return takes the words now, not a preset.
            highlighted = nil
            return .field
        case .focusCloseSurface(kind: .picker):
            guard isOpen else { return nil }
            forget()
            return .close
        default:
            return nil
        }
    }

    /// The field's words changed. Said to the controller unless they are
    /// what it put there itself. In the date field, typing takes the
    /// highlight away: Return means the words.
    public func typed(_ text: String) {
        guard isOpen, text != typed else { return }
        typed = text
        if field == .date { highlighted = nil }
        engine.focusPickerTyped(text)
    }

    /// ↑ or ↓: the next row, stopping at either end. From no highlight (the
    /// date field's), ↓ is the first row and ↑ the last.
    public func move(by delta: Int) {
        let tokens = rows.map(\.id)
        guard isOpen, !tokens.isEmpty, delta != 0 else { return }
        let next: Int
        if let at = highlighted.flatMap({ tokens.firstIndex(of: $0) }) {
            next = min(max(at + delta, 0), tokens.count - 1)
        } else {
            next = delta > 0 ? 0 : tokens.count - 1
        }
        highlighted = tokens[next]
    }

    /// Return. The label picker's is always the controller's confirm, which
    /// closes it (or makes and puts on the label typed), as its footnote
    /// says and GTK's did; elsewhere the highlighted row is chosen, and
    /// with none the controller takes the typed date.
    public func confirm() {
        guard isOpen else { return }
        if kind != .label, let highlighted {
            engine.focusPickerChoose(highlighted)
        } else {
            engine.invoke(PickerCommand.confirm)
        }
    }

    /// Space: the highlighted row toggled (a label on or off). The
    /// controller ignores it for a row that is not a label.
    public func toggle() {
        guard isOpen, let highlighted else { return }
        engine.focusPickerToggle(highlighted)
    }

    /// A click on the row `token`.
    public func choose(_ token: UInt64) {
        guard isOpen, rows.contains(where: { $0.id == token }) else { return }
        highlighted = token
        engine.focusPickerChoose(token)
    }

    /// Escape: the controller's Back, which closes the picker and says so.
    public func back() {
        engine.invoke(PickerCommand.back)
    }

    /// The toolkit closed the picker -- a click outside, the popover
    /// dismissing itself. `true` when it was open, and the controller must
    /// be told (`focusSurfaceClosed(.picker)`).
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        forget()
        return true
    }

    /// Run `command` if it is a picker key only Swift can answer, and say
    /// whether it was: `false` sends it on to `invoke`.
    public func run(_ command: String) -> Bool {
        guard isOpen else { return false }
        switch command {
        case PickerCommand.toggle: toggle()
        case PickerCommand.confirm: confirm()
        default: return false
        }
        return true
    }

    // MARK: -

    private func draw(_ view: PickerViewFfi, typedChanged: Bool) {
        let previous = rows
        let previousHighlight = highlighted
        kind = view.kind
        anchor = view.anchor
        title = view.title
        target = view.target
        field = view.field
        placeholder = view.placeholder
        typed = view.typed
        drawnFor = view.typed
        hint = view.hint
        footnote = view.footnote
        rows = view.rows.map { row in
            Row(
                id: row.token, section: row.section, name: row.name, detail: row.detail,
                cap: KeyCapSpelling.cap(row.key), dot: row.dot,
                colour: row.dot ? LabelColour(hex: row.color) : nil,
                applied: row.applied, create: row.create)
        }
        highlighted = highlight(after: previous, was: previousHighlight, typedChanged: typedChanged)
    }

    /// Where the highlight stands after a redraw. The tokens are new every
    /// time, so a kept highlight is kept by the row's name -- a label just
    /// toggled stays under the highlight -- then by its place. In the date
    /// field there is none until the arrows put one there; new words in a
    /// filter put it on the first row they leave.
    private func highlight(after previous: [Row], was: UInt64?, typedChanged: Bool) -> UInt64? {
        guard let first = rows.first else { return nil }
        if inField {
            guard let was, let at = previous.firstIndex(where: { $0.id == was }) else { return nil }
            return rows.first { $0.name == previous[at].name }?.id
        }
        guard !typedChanged, let was, let at = previous.firstIndex(where: { $0.id == was }) else {
            return first.id
        }
        if let same = rows.first(where: { $0.name == previous[at].name }) { return same.id }
        return rows[min(at, rows.count - 1)].id
    }

    private func forget() {
        isOpen = false
        inField = false
        typed = ""
        drawnFor = ""
        hint = nil
        rows = []
        highlighted = nil
    }
}
