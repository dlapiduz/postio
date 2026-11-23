import Foundation
import PostioFFI

public protocol FocusRowSource: AnyObject {
    var focusRowCount: UInt32 { get }
    func focusRow(at position: UInt32) -> FocusRowFfi?
    func residentFocusRow(at position: UInt32) -> FocusRowFfi?
}

/// Data, not a literal: the FFI hands the colour over as `#rrggbb`. A string that is not a colour
/// is no colour, and the dot is drawn plain rather than guessed.
public struct LabelColour: Equatable, Sendable {
    public let red: Double
    public let green: Double
    public let blue: Double

    public init(red: Double, green: Double, blue: Double) {
        self.red = red
        self.green = green
        self.blue = blue
    }

    nonisolated(unsafe) public static var accentHue: Double = 211

    public static func of(name: String, stored: String?) -> LabelColour? {
        LabelColour(hex: labelDotColour(name: name, stored: stored, accentHue: accentHue))
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

/// A shifted letter says its Shift (`⇧X`) because `e` and `E` are different keys, and a cap drawing
/// both as `E` would teach a different keyboard from the one the resolver reads. A sequence is one
/// cap, presses spaced (`g o`); a chord uses the Mac's glyphs; bare arrows run together. An FFI
/// gap: C22 asks for `postio_ui::hints::short` to spell every cap and the boundary does not export
/// it, so this is the one Swift spelling.
public enum KeyCapSpelling {
    public static func cap(_ binding: String?) -> String? {
        guard let binding, !binding.isEmpty else { return nil }
        let presses = binding.split(separator: " ").map { press -> [String] in
            let press = String(press)
            let halves = press.split(separator: "/").map(String.init)
            return (halves.count > 1 ? halves : [press]).map(key)
        }
        let keys = presses.flatMap { $0 }
        if keys.count > 1, keys.allSatisfy(arrows.contains) { return keys.joined() }
        return presses.map { $0.joined(separator: " ") }.joined(separator: " ")
    }

    private static let arrows: Set<String> = ["↑", "↓", "←", "→"]

    private static func key(_ press: String) -> String {
        if press.count == 1 {
            guard let only = press.first, only.isUppercase, only.isLetter else { return press }
            return "⇧" + press
        }
        switch press.lowercased() {
        case "space": return "Space"
        // Escape and Tab are words on a cap (as KEYS.md spells them); the menu's ⎋ and ⇥ are specks
        // at a cap's 10 pt. A chord (`shift+Tab`) keeps the glyphs.
        case "escape", "esc": return "Esc"
        case "tab": return "Tab"
        case "alt", "option": return "⌥"
        case "cmd", "command": return "⌘"
        case "shift": return "⇧"
        case "ctrl", "control": return "⌃"
        default: break
        }
        return MenuPlan.accelerator(from: press) ?? press
    }
}

public struct FocusRowModel: Equatable, Sendable {
    /// The row's shape is also its height: nothing is measured, so a 10k-row inbox scrolls without
    /// laying text out twice.
    public enum Layout: Equatable, Sendable {
        case oneLine
        case twoLine
        case digest
        case thread
        case child
    }

    public enum Chevron: Equatable, Sendable {
        case collapsed
        case expanded
    }

    public struct Child: Equatable, Sendable {
        public let initials: String
        public let sender: String
        public let unread: Bool
        public let tag: String?
        public let line: String
        public let placeholder: Bool
        public let attachment: Bool
        public let draft: Bool
        public let hintKeys: [String]
        public let hintLabel: String?
    }

    public struct Pill: Equatable, Sendable {
        public let name: String
        public let colour: LabelColour?
    }

    public struct Action: Equatable, Sendable {
        public let command: String
        public let label: String
        public let cap: String?
    }

    public struct Marker: Equatable, Sendable {
        public let chip: String
        public let date: String?
        public let quoted: String?
        public let status: String?
        public let actions: [Action]
    }

    public static let maxPills = 2

    public let id: Int64
    public let thread: Int64?
    public let layout: Layout
    public let sender: String
    public let fromAddress: String?
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
    public let newBadge: String?
    public let mutedBadge: String?
    public let chevron: Chevron?
    public let child: Child?
    public var heading: String?

    public init(_ row: FocusRowFfi, heading: String? = nil, binding: (String) -> String?) {
        id = row.id
        thread = row.thread
        layout = Self.layout(of: row)
        sender = row.threadRow?.participants ?? row.sender
        fromAddress = row.threadRow == nil ? row.fromAddress : nil
        subject = row.subject
        preview = row.threadRow.map { $0.latestLine.isEmpty ? nil : $0.latestLine } ?? row.preview
        newBadge = row.threadRow?.new
        mutedBadge = row.threadRow?.mutedBadge
        chevron = row.threadRow?.chevron.map { $0 == .expanded ? Chevron.expanded : .collapsed }
        child = row.child.map {
            Child(
                initials: $0.initials, sender: $0.sender, unread: $0.bold, tag: $0.tag,
                line: $0.line, placeholder: $0.placeholder, attachment: $0.attachment,
                draft: $0.draft, hintKeys: $0.hint?.keys.compactMap { KeyCapSpelling.cap($0) } ?? [],
                hintLabel: $0.hint?.label)
        }
        time = row.time
        dayHeading = row.dayHeading
        unread = row.unread
        countBadge = row.threadRow?.count ?? row.countBadge
        hasAttachments = row.hasAttachments
        sendState = row.sendState
        pills = row.pills.prefix(Self.maxPills).map {
            Pill(name: $0.name, colour: LabelColour.of(name: $0.name, stored: $0.color))
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

    public static func layout(of row: FocusRowFfi) -> Layout {
        if row.kind == .digest { return .digest }
        if row.kind == .child { return .child }
        if row.marker != nil { return .twoLine }
        return row.threadRow == nil ? .oneLine : .thread
    }

    public var bold: Bool { unread }

    public var dot: Bool { unread && layout != .digest && layout != .child }

    public var accessibilityLabel: String {
        if let child {
            var parts = [child.sender]
            if let tag = child.tag { parts.append(tag) }
            parts.append(child.line)
            parts.append(time)
            if child.unread { parts.append("unread") }
            return parts.joined(separator: ", ")
        }
        var parts = [sender, subject]
        if let count = countBadge, chevron != nil { parts.append("\(count) messages") }
        if let newBadge { parts.append(newBadge) }
        if let mutedBadge { parts.append(mutedBadge) }
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

/// An FFI gap: `postio_ui::focus_row::row_columns` is a rule the boundary does not export, so these
/// mirror its constants.
public enum FocusRowMetrics {
    public static let oneLine: Double = 40
    public static let twoLine: Double = 72
    public static let thread: Double = 46
    public static let child: Double = 38
    public static let dotCentre: Double = 15
    public static let chevronCentre: Double = 31
    public static let childLineX: Double = 25
    public static let childLineWidth: Double = 1.5
    public static let childAvatarX: Double = 69
    public static let childAvatar: Double = 20
    public static let heading: Double = 32
    public static let gutterCentre: Double = 30
    public static let firstLine: Double = 22
    public static let secondLine: Double = 51
    public static let senderX: Double = 56
    public static let senderWidth: Double = 222
    public static let senderMin: Double = 120
    public static let subjectRoom: Double = 260
    public static let gap: Double = 12
    public static let trailing: Double = 24

    public static func height(_ layout: FocusRowModel.Layout, headed: Bool) -> Double {
        let body: Double
        switch layout {
        case .twoLine: body = twoLine
        case .thread: body = thread
        case .child: return child
        case .oneLine, .digest: body = oneLine
        }
        return headed ? body + heading : body
    }

    public static func columns(width: Double) -> (senderWidth: Double, subjectX: Double, markerX: Double) {
        let spare = width - senderX - gap - subjectRoom - trailing
        let sender = min(max(spare, senderMin), senderWidth)
        let subjectX = senderX + sender + gap
        return (sender, subjectX, sender < senderWidth ? senderX : subjectX)
    }
}

/// Holds no array of rows, only what drawing needs without asking: each row's shape once seen, so
/// `heightOfRow` is answered without a page fetch, and the day each row is on. Day headings are a
/// band on a row, not rows of their own: separate rows would need every page's days to count the
/// table, and a 10k inbox would have to be read to be counted.
@MainActor
public final class FocusListModel {
    public static let pageSize: UInt32 = 50

    private let source: FocusRowSource
    private let binding: (String) -> String?

    public private(set) var count = 0

    public let focus: FocusIntents

    public var cursor: Int? { focus.cursor }

    public var selected: Set<Int64> { focus.selected }

    public func isPicked(_ message: Int64) -> Bool { focus.isPicked(message) }

    private struct Shape {
        let layout: FocusRowModel.Layout
        let day: String
    }
    private var shapes: [Int: Shape] = [:]

    public struct Splice: Equatable, Sendable {
        public let anchor: Int
        public let children: Int
        public var range: Range<Int> { (anchor + 1)..<(anchor + 1 + children) }
    }
    public private(set) var open: Splice?

    /// Whether the list was re-read while a thread was open and none has been opened since. The
    /// session draws its rows by its own splice, so by the time a re-read is drawn here `open`
    /// names the old thread; until a splice reconciles them the fold and splice redraw the list
    /// rather than slide rows (#2134).
    public private(set) var rereadWhileOpen = false

    /// A binding lookup clones the whole keymap on the other side, and the table draws a row's
    /// actions on every redraw.
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

    /// A re-read is not a new list: measured rows are measured again from whatever is resident
    /// before anything is drawn, since a table that forgot every shape laid out once at one-line
    /// heights. A shape whose row is not resident keeps its last answer; none is asked for, so
    /// nothing is paged.
    public func reset(total: UInt32) {
        count = Int(total)
        if open != nil { rereadWhileOpen = true }
        let known = shapes.keys.sorted()
        var kept: [Int: Shape] = [:]
        for index in known where index < count {
            if let row = source.residentFocusRow(at: UInt32(index)) {
                kept[index] = Shape(layout: FocusRowModel.layout(of: row), day: row.dayHeading)
            } else if let stale = shapes[index] {
                kept[index] = stale
            }
        }
        shapes = kept
        focus.listResized(count: count)
    }

    @discardableResult
    public func spliced(anchor: Int, children: Int) -> Range<Int> {
        _ = unspliced()
        let splice = Splice(anchor: anchor, children: children)
        shapes = Dictionary(
            uniqueKeysWithValues: shapes.map { $0.key > anchor ? ($0.key + children, $0.value) : ($0.key, $0.value) })
        count += children
        open = splice
        rereadWhileOpen = false
        return splice.range
    }

    @discardableResult
    public func unspliced() -> Range<Int>? {
        guard let splice = open else { return nil }
        let gone = splice.range
        shapes = Dictionary(
            uniqueKeysWithValues: shapes.compactMap { key, shape in
                if gone.contains(key) { return nil }
                return (key >= gone.upperBound ? key - splice.children : key, shape)
            })
        count -= splice.children
        open = nil
        return gone
    }

    private func display(row: Int) -> Int {
        guard let open, row > open.anchor else { return row }
        return row + open.children
    }

    public func keymapChanged() {
        caps = [:]
    }

    public func row(at index: Int) -> FocusRowModel? {
        guard index >= 0, index < count, let row = source.focusRow(at: UInt32(index)) else {
            return nil
        }
        learn(row, at: index)
        // On the same page the row before is resident; across a page boundary asking for it fetches
        // that page, and `pageArrived` redraws this row then.
        if index > 0, shapes[index - 1] == nil,
           let before = source.focusRow(at: UInt32(index - 1))
        {
            learn(before, at: index - 1)
        }
        return FocusRowModel(row, heading: heading(at: index, day: row.dayHeading)) { [self] command in
            lookUp(command)
        }
    }

    public func height(at index: Int) -> Double {
        let layout: FocusRowModel.Layout =
            open?.range.contains(index) == true ? .child : (shapes[index]?.layout ?? .oneLine)
        let headed = focus.heading != nil ? index == 0 : startsADay(index) ?? (index == 0)
        return FocusRowMetrics.height(layout, headed: headed)
    }

    private func heading(at index: Int, day: String) -> String? {
        if let single = focus.heading { return index == 0 ? single : nil }
        return startsADay(index) == true ? day : nil
    }

    public func pageArrived(_ page: UInt32) -> IndexSet {
        let rows = count - (open?.children ?? 0)
        let firstRow = Int(page * Self.pageSize)
        let endRow = min(firstRow + Int(Self.pageSize), rows)
        guard firstRow < endRow else { return [] }
        let first = display(row: firstRow)
        let end = display(row: endRow)
        for index in first..<end {
            if let row = source.focusRow(at: UInt32(index)) { learn(row, at: index) }
        }
        var changed = IndexSet(integersIn: first..<end)
        if end < count, shapes[end] != nil { changed.insert(end) }
        return changed
    }

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
