import Foundation
import PostioFFI
import Testing

@testable import PostioKit

@MainActor
struct PickerModelTests {
    final class Engine: PickerEngine {
        var typed: [String] = []
        var chosen: [UInt64] = []
        var toggled: [UInt64] = []
        var invoked: [String] = []
        var bindings: [String: String] = [:]

        func focusPickerTyped(_ text: String) { typed.append(text) }
        func focusPickerChoose(_ token: UInt64) { chosen.append(token) }
        func focusPickerToggle(_ token: UInt64) { toggled.append(token) }
        func invoke(_ id: String) { invoked.append(id) }
        func binding(for command: String) -> String? { bindings[command] }
    }

    static func row(
        _ token: UInt64, _ name: String, detail: String = "", key: String? = nil,
        section: String? = nil, dot: Bool = false, color: String? = nil, applied: Bool = false,
        create: Bool = false
    ) -> PickerRowFfi {
        PickerRowFfi(
            token: token, section: section, name: name, detail: detail, key: key, dot: dot,
            color: color, applied: applied, create: create)
    }

    static let presets: [PickerRowFfi] = [
        row(1, "Later today", detail: "18:00", key: "1"),
        row(2, "Tomorrow morning", detail: "Sun 27 Sep, 08:00", key: "2"),
        row(3, "Monday morning", detail: "Mon 28 Sep, 08:00", key: "3"),
        row(4, "Next week", detail: "Sat 3 Oct, 08:00", key: "4"),
    ]

    static let labels: [PickerRowFfi] = [
        row(11, "Atlas", detail: "\u{2713} applied", dot: true, color: "#c0703a", applied: true),
        row(12, "Harbor", detail: "71", dot: true),
        row(13, "Kitchen reno", detail: "42", dot: true, color: "#bb3322"),
    ]

    static func view(
        _ kind: PickerKindFfi = .snooze, anchor: PickerAnchorFfi = .row(position: 2),
        field: PickerFieldFfi? = nil, typed: String = "", hint: String? = nil,
        rows: [PickerRowFfi]? = nil
    ) -> PickerViewFfi {
        let dated = kind == .snooze || kind == .remind
        return PickerViewFfi(
            kind: kind, anchor: anchor,
            title: dated ? "Snooze until" : "Labels",
            target: "Ada Moreno \u{b7} Atlas Q3 budget",
            field: field ?? (dated ? .date : .filter),
            placeholder: dated ? "Or type a date: \u{201c}tue 9am\u{201d}" : "Filter, or type a new label",
            typed: typed,
            hint: hint ?? (dated ? "Tab to type" : nil),
            rows: rows ?? (dated ? presets : labels),
            footnote: "Snoozed mail is under g z.")
    }

    static func model() -> (PickerModel, Engine) {
        let engine = Engine()
        return (PickerModel(engine: engine), engine)
    }

    @Test func theSnoozePresetsAreNumberedOneToFourWithKeycaps() {
        let (picker, _) = Self.model()
        let change = picker.apply(.focusOpenPicker(view: Self.view()))
        #expect(change == .open(anchor: .row(position: 2)))
        #expect(picker.isOpen)
        #expect(picker.kind == .snooze)
        #expect(picker.title == "Snooze until")
        #expect(picker.target == "Ada Moreno \u{b7} Atlas Q3 budget")
        #expect(picker.rows.map(\.name) == ["Later today", "Tomorrow morning", "Monday morning", "Next week"])
        #expect(picker.rows.map(\.cap) == ["1", "2", "3", "4"])
        #expect(picker.rows.map(\.detail) == ["18:00", "Sun 27 Sep, 08:00", "Mon 28 Sep, 08:00", "Sat 3 Oct, 08:00"])
        #expect(picker.highlighted == 1)
        #expect(picker.hint == "Tab to type")
        #expect(picker.footnote == "Snoozed mail is under g z.")
    }

    @Test func aLabelsDotIsItsOwnColourOrOneFromItsName() {
        let (picker, _) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view(.label)))
        #expect(picker.rows.map(\.dot) == [true, true, true])
        #expect(picker.rows[0].colour == LabelColour(hex: "#c0703a"))
        #expect(picker.rows[1].colour == LabelColour.of(name: picker.rows[1].name, stored: nil))
        #expect(picker.rows[1].colour != nil)
        #expect(picker.rows[0].applied)
        #expect(picker.rows.allSatisfy { $0.cap == nil })
    }

    @Test func aHeadingIsDrawnWhereTheControllerStartsASection() {
        let (picker, _) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view(.move, rows: [])))
        #expect(picker.rows.isEmpty)
        #expect(picker.highlighted == nil)
        picker.apply(.focusPickerRows(view: Self.view(.move, rows: [
            Self.row(21, "Receipts", detail: "214", key: "1", section: "Recent"),
            Self.row(22, "Travel", detail: "18", key: "2"),
            Self.row(23, "Archive", detail: "18,204", section: "All folders"),
        ])))
        #expect(picker.rows.map(\.section) == ["Recent", nil, "All folders"])
        #expect(picker.highlighted == 21)
    }

    @Test func eventsThatAreNotThePickersChangeNothing() {
        let (picker, _) = Self.model()
        #expect(picker.apply(.focusCloseSurface(kind: .bar)) == nil)
        #expect(picker.apply(.focusPickerRows(view: Self.view())) == nil)
        #expect(!picker.isOpen)
        #expect(picker.apply(.focusPickerField) == nil)
    }

    @Test func tabPutsTheKeyboardInTheDateFieldAndReturnTakesTheTypedDate() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        #expect(!picker.inField)
        #expect(picker.apply(.focusPickerField) == .field)
        #expect(picker.inField)
        #expect(picker.highlighted == nil)
        picker.typed("tue 9am")
        #expect(engine.typed == ["tue 9am"])
        #expect(picker.typed == "tue 9am")
        picker.apply(.focusPickerRows(view: Self.view(typed: "tue 9am", hint: "Tue 29 Sep, 09:00")))
        #expect(picker.hint == "Tue 29 Sep, 09:00")
        #expect(picker.highlighted == nil)
        picker.confirm()
        #expect(engine.invoked == [PickerCommand.confirm])
        #expect(engine.chosen.isEmpty)
    }

    @Test func theFieldsOwnWordsAreNotSaidBack() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view(.label, typed: "")))
        picker.typed("")
        #expect(engine.typed.isEmpty)
        picker.typed("Rec")
        picker.apply(.focusPickerRows(view: Self.view(.label, typed: "Rec")))
        picker.typed("Rec")
        #expect(engine.typed == ["Rec"])
    }

    @Test func aFilterTypedPutsTheHighlightOnTheFirstRowItLeaves() {
        let (picker, _) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view(.label)))
        picker.move(by: 2)
        #expect(picker.highlighted == 13)
        picker.typed("Rec")
        picker.apply(.focusPickerRows(view: Self.view(.label, typed: "Rec", rows: [
            Self.row(31, "Create label \u{201c}Rec\u{201d}", create: true),
            Self.row(32, "Receipts", detail: "4", dot: true),
        ])))
        #expect(picker.highlighted == 31)
        #expect(picker.rows[0].create)
    }

    @Test func spaceTogglesTheHighlightedLabelAndTheHighlightStaysOnIt() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view(.label)))
        picker.move(by: 1)
        picker.toggle()
        #expect(engine.toggled == [12])
        picker.apply(.focusPickerRows(view: Self.view(.label, rows: [
            Self.row(41, "Atlas", detail: "\u{2713} applied", dot: true, applied: true),
            Self.row(42, "Harbor", detail: "\u{2713} applied", dot: true, applied: true),
            Self.row(43, "Kitchen reno", detail: "42", dot: true),
        ])))
        #expect(picker.highlighted == 42)
        #expect(picker.rows[1].applied)
    }

    @Test func returnChoosesTheHighlightedPreset() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        picker.move(by: 1)
        picker.confirm()
        #expect(engine.chosen == [2])
        #expect(engine.invoked.isEmpty)
    }

    @Test func returnInTheLabelPickerClosesItAsItsFootnoteSays() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view(.label)))
        picker.confirm()
        #expect(engine.invoked == [PickerCommand.confirm])
        #expect(engine.chosen.isEmpty)
        #expect(engine.toggled.isEmpty)
    }

    @Test func aClickChoosesItsOwnRow() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        picker.choose(3)
        #expect(engine.chosen == [3])
        #expect(picker.highlighted == 3)
        picker.choose(99)
        #expect(engine.chosen == [3])
    }

    @Test func escapeIsTheControllersBackAndThePickerClosesWhenItSaysSo() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        picker.back()
        #expect(engine.invoked == ["back"])
        #expect(picker.isOpen)
        #expect(picker.apply(.focusCloseSurface(kind: .picker)) == .close)
        #expect(!picker.isOpen)
        #expect(picker.rows.isEmpty)
        #expect(picker.closedByToolkit() == false)
    }

    @Test func aClickOutsideIsTheToolkitsCloseAndIsSaidOnce() {
        let (picker, _) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        #expect(picker.closedByToolkit())
        #expect(!picker.isOpen)
        #expect(picker.closedByToolkit() == false)
        #expect(picker.apply(.focusCloseSurface(kind: .picker)) == nil)
    }

    @Test func theKeysOnlySwiftCanAnswerAreRunHere() {
        let (picker, engine) = Self.model()
        #expect(picker.run(PickerCommand.toggle) == false)
        picker.apply(.focusOpenPicker(view: Self.view(.label)))
        #expect(picker.run(PickerCommand.toggle))
        #expect(engine.toggled == [11])
        #expect(picker.run(PickerCommand.confirm))
        #expect(engine.invoked == [PickerCommand.confirm])
        #expect(picker.run("picker_choose_1") == false)
        #expect(picker.run("back") == false)
    }

    @Test func theArrowsMoveTheHighlightAndStopAtEitherEnd() {
        let (picker, _) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        picker.move(by: 1)
        #expect(picker.highlighted == 2)
        picker.move(by: 5)
        #expect(picker.highlighted == 4)
        picker.move(by: -9)
        #expect(picker.highlighted == 1)
    }

    @Test func theArrowsFromTheDateFieldComeBackToThePresets() {
        let (picker, engine) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        picker.apply(.focusPickerField)
        picker.move(by: -1)
        #expect(picker.highlighted == 4)
        picker.confirm()
        #expect(engine.chosen == [4])
    }

    @Test func aNewPickerForgetsTheOldOne() {
        let (picker, _) = Self.model()
        picker.apply(.focusOpenPicker(view: Self.view()))
        picker.apply(.focusPickerField)
        picker.move(by: 1)
        picker.apply(.focusOpenPicker(view: Self.view(.label, anchor: .openMessage)))
        #expect(picker.kind == .label)
        #expect(picker.anchor == .openMessage)
        #expect(!picker.inField)
        #expect(picker.highlighted == 11)
    }
}
