import Foundation
import PostioFFI

/// Where Focus's rows come from.
///
/// A protocol so the list can be driven by something other than a live
/// session: nothing in a unit test can open a store, and everything this
/// file decides -- which row starts a day, what a page arriving changes --
/// is worth proving without one. `PostioSession` is the real one.
public protocol FocusRowSource: AnyObject {
    /// How many rows the list draws. A count on the other side, never the
    /// length of anything held here.
    var focusRowCount: UInt32 { get }
    /// The row at `position`, or `nil` while its page is on its way
    /// (`FocusPageReady` follows). Synchronous, and no I/O.
    func focusRow(at position: UInt32) -> FocusRowFfi?
}

/// A label's own colour, as the user gave it.
///
/// Data, not a literal: the FFI hands the label's colour over as `#rrggbb`,
/// and the pill's dot is the one place a hue other than the accent is drawn
/// (the Mac pack's "Label dots are the only other hues"). A string that is
/// not a colour is no colour, and the dot is drawn plain rather than guessed.
public struct LabelColour: Equatable, Sendable {
    public let red: Double
    public let green: Double
    public let blue: Double

    public init(red: Double, green: Double, blue: Double) {
        self.red = red
        self.green = green
        self.blue = blue
    }

    public init?(hex: String?) {
        guard let hex, hex.count == 7, hex.first == "#",
              let value = UInt32(hex.dropFirst(), radix: 16)
        else { return nil }
        red = Double((value >> 16) & 0xff) / 255
        green = Double((value >> 8) & 0xff) / 255
        blue = Double(value & 0xff) / 255
    }
}

/// A binding as the keycap that shows it.
///
/// The keymap's own keys (spec 007 C22): `e` and `E` are two different
/// keys, and a cap that drew both as `E` would be teaching a different
/// keyboard from the one the resolver reads. So a shifted letter says its
/// Shift, as the design writes it (`⇧X`, `⇧J ⇧K`, KEYS.md), and a plain
/// one is itself. A sequence is one cap, its presses spaced (`g o`), as
/// screen 01 draws it. A chord is drawn with the Mac's glyphs, as the menu
/// draws it (`⌘K`). A pair (`hints::pair`'s `j/k`) is its two keys spaced
/// (`j k`, `] [`), and bare arrows, pair or sequence, are one run of glyphs
/// (`↑↓`, `↑↓←→`), as the design's footers draw them.
///
/// **An FFI gap, said here so it is found.** C22 asks for the shared hint
/// code (`postio_ui::hints::short`) to spell every cap, and the boundary
/// does not export it; until it does this is the one Swift spelling, and
/// every cap on the Mac goes through it.
public enum KeyCapSpelling {
    public static func cap(_ binding: String?) -> String? {
        guard let binding, !binding.isEmpty else { return nil }
        let presses = binding.split(separator: " ").map { press -> [String] in
            let press = String(press)
            // A pair: each key spelled on its own.
            // `alt+/` is one key, the slash itself, not a pair.
            let halves = press.split(separator: "/").map(String.init)
            return (halves.count > 1 ? halves : [press]).map(key)
        }
        let keys = presses.flatMap { $0 }
        if keys.count > 1, keys.allSatisfy(arrows.contains) { return keys.joined() }
        return presses.map { $0.joined(separator: " ") }.joined(separator: " ")
    }

    /// The arrows' glyphs, which run together when they stand alone.
    private static let arrows: Set<String> = ["↑", "↓", "←", "→"]

    /// One key as its cap spells it.
    private static func key(_ press: String) -> String {
        if press.count == 1 {
            // A folded Shift (`X` is `shift+x`) says so.
            guard let only = press.first, only.isUppercase, only.isLetter else { return press }
            return "⇧" + press
        }
        // A popover's own keys (`popover_hints`): Space as its cap says
        // it, a modifier held for a click as its glyph.
        switch press.lowercased() {
        case "space": return "Space"
        case "alt", "option": return "⌥"
        case "cmd", "command": return "⌘"
        case "shift": return "⇧"
        case "ctrl", "control": return "⌃"
        default: break
        }
        return MenuPlan.accelerator(from: press) ?? press
    }
}

/// One row of Focus's list, as the Mac lays it out.
///
/// Built from `FocusRowFfi`, which already carries every word the row
/// says: the presenters GTK's row uses made them, so the two apps cannot
/// word a row differently. What is decided here is only layout -- which
/// of the three shapes a row takes, which pills fit, which keycap an
/// action draws.
public struct FocusRowModel: Equatable, Sendable {
    /// The row's shape, which is also its height: nothing about a row is
    /// measured, so a 10k-row inbox scrolls without laying text out twice.
    public enum Layout: Equatable, Sendable {
        /// Sender, subject, pills, preview, count, time.
        case oneLine
        /// A marked row: the one line, then the marker's.
        case twoLine
        /// A digest delivery: the stacked icon, one line.
        case digest
    }

    /// A label pill: its name, and the dot in the label's own colour.
    public struct Pill: Equatable, Sendable {
        public let name: String
        public let colour: LabelColour?
    }

    /// One answer on the marker's line, with the key that does the same.
    public struct Action: Equatable, Sendable {
        /// The registry command it runs.
        public let command: String
        /// The button's words, from the FFI.
        public let label: String
        /// The keycap, from the user's binding; `nil` when it has none.
        public let cap: String?
    }

    /// A marked row's second line.
    public struct Marker: Equatable, Sendable {
        /// "Invite", "Question", "To-do", "No reply".
        public let chip: String
        /// "Tue 29 Sep · 10:00–10:45".
        public let date: String?
        /// The sentence, verbatim, inside quotation marks -- as GTK's row
        /// draws it.
        public let quoted: String?
        /// What stands where the actions would: "Accepted", "Past".
        public let status: String?
        /// The answers, in order.
        public let actions: [Action]
    }

    /// The most pills a row draws (`postio_ui::focus_row::MAX_PILLS`).
    public static let maxPills = 2

    public let id: Int64
    public let thread: Int64?
    public let layout: Layout
    public let sender: String
    public let subject: String
    public let preview: String?
    public let time: String
    public let dayHeading: String
    public let unread: Bool
    public let countBadge: String?
    public let hasAttachments: Bool
    public let sendState: String?
    public let pills: [Pill]
    public let marker: Marker?
    /// The day heading drawn above this row, when it is the first of its
    /// day. Set by `FocusListModel`, the only thing that knows the row
    /// before.
    public var heading: String?

    public init(_ row: FocusRowFfi, heading: String? = nil, binding: (String) -> String?) {
        id = row.id
        thread = row.thread
        layout = Self.layout(of: row)
        sender = row.sender
        subject = row.subject
        preview = row.preview
        time = row.time
        dayHeading = row.dayHeading
        unread = row.unread
        countBadge = row.countBadge
        hasAttachments = row.hasAttachments
        sendState = row.sendState
        pills = row.pills.prefix(Self.maxPills).map {
            Pill(name: $0.name, colour: LabelColour(hex: $0.color))
        }
        marker = row.marker.map { line in
            Marker(
                chip: line.chip,
                date: line.date,
                quoted: line.quote.map { "\u{201c}\($0)\u{201d}" },
                status: line.status,
                actions: line.actions.map {
                    Action(
                        command: $0.command,
                        label: $0.label,
                        cap: KeyCapSpelling.cap(binding($0.command))
                    )
                }
            )
        }
        self.heading = heading
    }

    /// The shape a row takes, from its kind and whether it is marked.
    public static func layout(of row: FocusRowFfi) -> Layout {
        if row.kind == .digest { return .digest }
        return row.marker == nil ? .oneLine : .twoLine
    }

    /// Unread is bold (the Mac pack's global rules).
    public var bold: Bool { unread }

    /// What VoiceOver says for the row: GTK's row says the same things in
    /// the same order (`postio-gtk` `list/row.rs`).
    public var accessibilityLabel: String {
        var parts = [sender, subject]
        if let preview, !preview.isEmpty { parts.append(preview) }
        if unread { parts.append("unread") }
        if let sendState { parts.append(sendState) }
        if let marker {
            var said = marker.chip
            if let date = marker.date { said += " \(date)" }
            if let quoted = marker.quoted { said += ": \(quoted)" }
            if let status = marker.status { said += ", \(status)" }
            parts.append(said)
        }
        return parts.joined(separator: ", ")
    }
}

/// The list's geometry, from screen 01 -- the same numbers GTK's row draws
/// with (`postio-gtk` `list/row.rs`, `postio_ui::focus_row`).
///
/// **An FFI gap.** `postio_ui::focus_row::row_columns` -- where the sender's
/// column narrows -- is a rule, and the boundary does not export it; these
/// mirror its constants until it does.
public enum FocusRowMetrics {
    /// A one-line row (and a digest's).
    public static let oneLine: Double = 40
    /// A marked row.
    public static let twoLine: Double = 72
    /// The day heading's band above the first row of a day.
    public static let heading: Double = 32
    /// The centre of the gutter's dot, box or icon.
    public static let gutterCentre: Double = 30
    /// The centres of a two-line row's lines.
    public static let firstLine: Double = 22
    public static let secondLine: Double = 51
    /// Where the sender's column starts, and how wide it is at most and at
    /// least (`SENDER_X`, `SENDER_WIDTH`, `SENDER_MIN`).
    public static let senderX: Double = 56
    public static let senderWidth: Double = 222
    public static let senderMin: Double = 120
    /// The room the subject keeps before the sender gives any up.
    public static let subjectRoom: Double = 260
    /// The gap between things on a line, and the room the right edge keeps.
    public static let gap: Double = 12
    public static let trailing: Double = 24

    /// A row's height: its layout's, plus the heading's band when it starts
    /// a day. Fixed per shape, so nothing is ever measured.
    public static func height(_ layout: FocusRowModel.Layout, headed: Bool) -> Double {
        let body = layout == .twoLine ? twoLine : oneLine
        return headed ? body + heading : body
    }

    /// Where the subject starts in a row `width` wide, and where a marked
    /// row's second line does (under the subject, or under the sender once
    /// the sender's column has narrowed).
    public static func columns(width: Double) -> (senderWidth: Double, subjectX: Double, markerX: Double) {
        let spare = width - senderX - gap - subjectRoom - trailing
        let sender = min(max(spare, senderMin), senderWidth)
        let subjectX = senderX + sender + gap
        return (sender, subjectX, sender < senderWidth ? senderX : subjectX)
    }
}

/// Focus's list, as the table reads it.
///
/// The count and the rows are the engine's: this holds no array of rows,
/// only what drawing needs without asking -- each row's shape once it has
/// been seen, so `heightOfRow` (which `NSTableView` asks for every row) is
/// answered without a page fetch, and the day each row is on, so a row
/// knows whether it starts one.
///
/// **Day headings are a band on a row, not rows of their own.** A heading
/// is drawn over the first row of its day, so table rows stay one-to-one
/// with the FFI's positions. Headings as separate rows would need every
/// page's days to know how many rows the table has, and a 10k inbox would
/// have to be read to be counted.
@MainActor
public final class FocusListModel {
    /// The FFI's page size: `FocusPageReady(page)` covers this many rows.
    public static let pageSize: UInt32 = 50

    private let source: FocusRowSource
    private let binding: (String) -> String?

    /// How many rows the list draws.
    public private(set) var count = 0

    /// The controller's cursor, selection and heading, as its intents left
    /// them (`FocusIntents`). Read here; written only by the intents.
    public let focus: FocusIntents

    /// Where the keyboard is: the first row when the list opens (C30), and
    /// nothing in an empty list. The controller's (`FocusIntents`).
    public var cursor: Int? { focus.cursor }

    /// The messages marked, drawn as the checked box in the gutter.
    public var selected: Set<Int64> { focus.selected }

    /// Whether the row for `message` draws the checked box: marked by id,
    /// or everything selected (C19).
    public func isPicked(_ message: Int64) -> Bool { focus.isPicked(message) }

    /// What has been learned about a row: its shape, and its day.
    private struct Shape {
        let layout: FocusRowModel.Layout
        let day: String
    }
    private var shapes: [Int: Shape] = [:]

    /// Keycaps by command, until the keymap changes. A binding lookup
    /// clones the whole keymap on the other side, and the table draws a
    /// row's actions on every redraw.
    private var caps: [String: String?] = [:]

    public init(
        source: FocusRowSource,
        focus: FocusIntents = FocusIntents(),
        binding: @escaping (String) -> String?
    ) {
        self.source = source
        self.focus = focus
        self.binding = binding
    }

    /// The list changed over, was re-read, or changed length
    /// (`FocusListChanged`): everything learned is about a list that is
    /// gone.
    public func reset(total: UInt32) {
        count = Int(total)
        shapes = [:]
        // A re-read keeps the keyboard where it was, inside the list; an
        // empty list has nowhere for it to be.
        focus.listResized(count: count)
    }

    /// `[keys]` changed: every cap may spell something else now.
    public func keymapChanged() {
        caps = [:]
    }

    /// The row at `index`, or `nil` while its page is on its way.
    public func row(at index: Int) -> FocusRowModel? {
        guard index >= 0, index < count, let row = source.focusRow(at: UInt32(index)) else {
            return nil
        }
        learn(row, at: index)
        // The row before decides whether this one starts a day. On the
        // same page it is resident; across a page boundary asking for it is
        // what fetches that page, and `pageArrived` redraws this row then.
        if index > 0, shapes[index - 1] == nil,
           let before = source.focusRow(at: UInt32(index - 1))
        {
            learn(before, at: index - 1)
        }
        return FocusRowModel(row, heading: heading(at: index, day: row.dayHeading)) { [self] command in
            lookUp(command)
        }
    }

    /// The row's height, from what is known of it, without asking the
    /// engine. A row not seen yet is drawn as a one-liner until its page
    /// says otherwise.
    public func height(at index: Int) -> Double {
        let layout = shapes[index]?.layout ?? .oneLine
        let headed = focus.heading != nil ? index == 0 : startsADay(index) ?? (index == 0)
        return FocusRowMetrics.height(layout, headed: headed)
    }

    /// The heading drawn over the row at `index`: `!`'s one heading over
    /// the first row while it narrows the list (screen 03), else the day's
    /// over the first row of each day.
    private func heading(at index: Int, day: String) -> String? {
        if let single = focus.heading { return index == 0 ? single : nil }
        return startsADay(index) == true ? day : nil
    }

    /// A page landed: learn its rows, and say which rows to redraw -- the
    /// page's own, and the next page's first, whose heading depended on
    /// this page's last row.
    public func pageArrived(_ page: UInt32) -> IndexSet {
        let first = Int(page * Self.pageSize)
        let end = min(first + Int(Self.pageSize), count)
        guard first < end else { return [] }
        for index in first..<end {
            if let row = source.focusRow(at: UInt32(index)) { learn(row, at: index) }
        }
        var changed = IndexSet(integersIn: first..<end)
        if end < count, shapes[end] != nil { changed.insert(end) }
        return changed
    }

    /// Whether the row at `index` is the first of its day: `nil` while that
    /// cannot be known, because one of the two rows has not been seen.
    private func startsADay(_ index: Int) -> Bool? {
        if index == 0 { return true }
        guard let this = shapes[index], let before = shapes[index - 1] else { return nil }
        return this.day != before.day
    }

    private func learn(_ row: FocusRowFfi, at index: Int) {
        shapes[index] = Shape(layout: FocusRowModel.layout(of: row), day: row.dayHeading)
    }

    private func lookUp(_ command: String) -> String? {
        if let known = caps[command] { return known }
        let found = binding(command)
        caps[command] = found
        return found
    }
}
