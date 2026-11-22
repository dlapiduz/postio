import Foundation
import Observation
import PostioFFI

/// Every call is cheap: the controller answers in memory and says what changed as `UiEvent`s.
public protocol PickerEngine: AnyObject {
    func focusPickerTyped(_ text: String)
    func focusPickerChoose(_ token: UInt64)
    func focusPickerToggle(_ token: UInt64)
    func invoke(_ id: String)
    func binding(for command: String) -> String?
}

extension PostioSession: PickerEngine {}

/// Commands about the highlighted row, which only the toolkit knows. Every other picker key goes to
/// `invoke` as resolved.
public enum PickerCommand {
    public static let toggle = "picker_toggle"
    public static let confirm = "picker_confirm"
    public static let back = "back"
    /// Where a picker hangs when its own verb has folded into More.
    public static let more = "more_actions"

    public static func opening(_ kind: PickerKindFfi) -> String {
        switch kind {
        case .snooze: return "snooze"
        case .remind: return "remind_if_no_reply"
        case .label: return "add_label"
        case .move: return "move"
        }
    }
}

/// The controller owns which picker opens and what choosing does. The highlight is decided here:
/// arrows move it, a redraw keeps it, and typing a date clears it (Return then means the date). No
/// AppKit (#1264).
@MainActor
@Observable
public final class PickerModel {
    public enum Change: Equatable {
        case open(anchor: PickerAnchorFfi)
        case rows
        case field
        case close
    }

    public struct Row: Equatable, Identifiable {
        public let id: UInt64
        public let section: String?
        public let name: String
        public let detail: String
        public let cap: String?
        public let dot: Bool
        /// `nil` for the secondary colour (the boundary does not export `postio_ui::label_colour`).
        public let colour: LabelColour?
        public let applied: Bool
        public let create: Bool
    }

    public private(set) var isOpen = false
    public private(set) var kind: PickerKindFfi = .snooze
    public private(set) var anchor: PickerAnchorFfi = .openMessage
    public private(set) var title = ""
    public private(set) var target = ""
    public private(set) var field: PickerFieldFfi = .date
    public private(set) var placeholder = ""
    public private(set) var typed = ""
    public private(set) var hint: String?
    public private(set) var rows: [Row] = []
    public private(set) var footnote = ""
    public private(set) var highlighted: UInt64?
    public private(set) var inField = false

    @ObservationIgnored private let engine: PickerEngine
    /// New words in a filter move the highlight to the first row they leave.
    @ObservationIgnored private var drawnFor = ""

    public init(engine: PickerEngine) {
        self.engine = engine
    }

    public var filters: Bool { field == .filter }

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

    /// Not sent to the controller when they are what it put there itself. Typing in the date field
    /// clears the highlight.
    public func typed(_ text: String) {
        guard isOpen, text != typed else { return }
        typed = text
        if field == .date { highlighted = nil }
        engine.focusPickerTyped(text)
    }

    /// From no highlight (the date field's), down is the first row and up the last.
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

    /// The label picker's Return is always the controller's confirm (closes it, or makes and
    /// applies the typed label); elsewhere it chooses the highlighted row, and with none takes the
    /// typed date.
    public func confirm() {
        guard isOpen else { return }
        if kind != .label, let highlighted {
            engine.focusPickerChoose(highlighted)
        } else {
            engine.invoke(PickerCommand.confirm)
        }
    }

    public func toggle() {
        guard isOpen, let highlighted else { return }
        engine.focusPickerToggle(highlighted)
    }

    public func choose(_ token: UInt64) {
        guard isOpen, rows.contains(where: { $0.id == token }) else { return }
        highlighted = token
        engine.focusPickerChoose(token)
    }

    public func back() {
        engine.invoke(PickerCommand.back)
    }

    /// `true` when it was open, in which case the controller must be told
    /// (`focusSurfaceClosed(.picker)`).
    public func closedByToolkit() -> Bool {
        guard isOpen else { return false }
        forget()
        return true
    }

    public func run(_ command: String) -> Bool {
        guard isOpen else { return false }
        switch command {
        case PickerCommand.toggle: toggle()
        case PickerCommand.confirm: confirm()
        default: return false
        }
        return true
    }

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
                colour: row.dot ? LabelColour.of(name: row.name, stored: row.color) : nil,
                applied: row.applied, create: row.create)
        }
        highlighted = highlight(after: previous, was: previousHighlight, typedChanged: typedChanged)
    }

    /// A kept highlight is kept by the row's name (tokens are new every redraw), then by its place.
    /// None in the date field until the arrows put one there.
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
