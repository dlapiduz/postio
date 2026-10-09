import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// One query, two controls (specs/010-focus-search T067; design §1 and
/// §3.2): every filter is both a chip in the field and a button in the
/// filter bar, and changing either changes the other.
///
/// Not because Swift keeps them in step: the controller does
/// (`crates/postio-focus`, `edit::apply`). A control's change crosses as a
/// `TermEditFfi` -- an operator's keyword and its value, or a chip's token
/// -- and the next `FocusQuery` redraws both the chips and the buttons.
/// Swift never builds query text: no `field:value` is ever spelled here.
@MainActor
struct SearchQuerySyncTests {
    final class Engine: SearchQueryEngine {
        var edits: [TermEditFfi] = []
        var opened: [FilterKindFfi] = []
        func focusSearchEdit(_ edit: TermEditFfi) { edits.append(edit) }
        func focusSearchPopover(_ kind: FilterKindFfi) { opened.append(kind) }
    }

    static func button(_ kind: FilterKindFfi, _ label: String, applied: Bool = false) -> FilterButtonFfi {
        FilterButtonFfi(kind: kind, label: label, applied: applied, open: false)
    }

    /// The bar's eight buttons, `applied` the ones the query holds, with
    /// the labels the controller gives them.
    static func buttons(_ applied: [FilterKindFfi: String] = [:]) -> [FilterButtonFfi] {
        let plain: [(FilterKindFfi, String)] = [
            (.from, "From"), (.to, "To"), (.date, "Date"), (.anywhere, "Anywhere"), (.label, "Label"),
            (.attachment, "Attachment"), (.hasAction, "Has action"), (.unread, "Unread"),
        ]
        return plain.map { kind, label in
            button(kind, applied[kind] ?? label, applied: applied[kind] != nil)
        }
    }

    static func chip(_ token: UInt32, _ op: String, _ value: String, excluded: Bool = false) -> QueryChipFfi {
        QueryChipFfi(token: token, operator: op, value: value, excluded: excluded, focused: false)
    }

    static func query(_ chips: [QueryChipFfi], words: String = "atlas budget", applied: [FilterKindFfi: String] = [:])
        -> UiEvent
    {
        .focusQuery(
            view: QueryViewFfi(
                chips: chips, words: words, hint: "/ to edit", hintKey: nil, ringed: false, buttons: buttons(applied)))
    }

    static func applied(_ model: SearchQueryModel, _ kind: FilterKindFfi) -> SearchQueryModel.Button? {
        model.buttons.first { $0.kind == kind }
    }

    @Test func tappingAttachmentSendsItsToggle() {
        let engine = Engine()
        let model = SearchQueryModel(engine: engine)
        model.apply(Self.query([]))

        model.tap(.attachment)

        #expect(engine.edits == [.toggle(field: "has", value: "attachment")])
    }

    @Test func theNextQueryShowsTheChipAndTheSolidButton() {
        let model = SearchQueryModel(engine: Engine())
        model.apply(Self.query([]))
        #expect(Self.applied(model, .attachment)?.applied == false)

        let change = model.apply(Self.query(
            [Self.chip(7, "has:", "attachment")], applied: [.attachment: "Attachment"]))

        #expect(change == .query)
        #expect(model.chips.map(\.id) == [7])
        #expect(model.chips.first?.op == "has:")
        #expect(model.chips.first?.value == "attachment")
        #expect(Self.applied(model, .attachment)?.applied == true)
        #expect(model.words == "atlas budget")
    }

    @Test func removingAChipSendsItsTokenAndTheButtonGoesBackToOutline() {
        let engine = Engine()
        let model = SearchQueryModel(engine: engine)
        model.apply(Self.query([Self.chip(7, "has:", "attachment")], applied: [.attachment: "Attachment"]))

        model.remove(7)
        #expect(engine.edits == [.remove(token: 7)])

        model.apply(Self.query([]))
        #expect(model.chips.isEmpty)
        #expect(Self.applied(model, .attachment)?.applied == false)
    }

    @Test func aTypedFromArrivingFromRustMakesFromSolidWithItsLabel() {
        let model = SearchQueryModel(engine: Engine())
        model.apply(Self.query([]))

        // Typed in the field as words; the controller lowered it to a chip
        // and named the button.
        model.apply(Self.query([Self.chip(3, "from:", "ada")], words: "", applied: [.from: "From: Ada Moreno"]))

        let from = Self.applied(model, .from)
        #expect(from?.applied == true)
        #expect(from?.label == "From: Ada Moreno")
        #expect(model.words.isEmpty)
    }

    @Test func swiftNeverBuildsQueryText() {
        let engine = Engine()
        let model = SearchQueryModel(engine: engine)
        model.apply(Self.query([Self.chip(3, "from:", "ada"), Self.chip(4, "is:", "unread")]))

        for kind in [FilterKindFfi.attachment, .hasAction, .unread] { model.tap(kind) }
        model.remove(3)
        model.clearFilters()

        // Every edit is a keyword and a value apart, or a token: nothing
        // joined into query text on this side.
        for edit in engine.edits {
            switch edit {
            case let .toggle(field, value), let .add(field, value, _):
                #expect(!field.contains(":") && !field.contains(" "))
                #expect(!value.contains(":"))
            case .remove, .setMonths, .clearFilters:
                break
            }
        }
        #expect(engine.edits == [
            .toggle(field: "has", value: "attachment"),
            .toggle(field: "has", value: "action"),
            .toggle(field: "is", value: "unread"),
            .remove(token: 3),
            .clearFilters,
        ])
    }

    @Test func aPopoverButtonAsksForItsPopoverAndEditsNothing() {
        // From, To, Date, Anywhere and Label open popovers (step 4): a
        // click asks the controller for one, and the query changes only
        // when something in it is checked.
        let engine = Engine()
        let model = SearchQueryModel(engine: engine)
        model.apply(Self.query([]))

        for kind in [FilterKindFfi.from, .to, .date, .anywhere, .label] { model.tap(kind) }

        #expect(engine.edits.isEmpty)
        #expect(engine.opened == [.from, .to, .date, .anywhere, .label])
    }

    /// Screen 13 (D24): with nothing found the hint names ⌘⌫ as its cap,
    /// and the whole field is ringed, as the controller says.
    @Test func aHintWithAKeySpellsTheKeyAsItsCap() {
        let model = SearchQueryModel(engine: Engine())
        model.apply(
            .focusQuery(
                view: QueryViewFfi(
                    chips: [], words: "", hint: "clears filters", hintKey: "cmd+BackSpace", ringed: true,
                    buttons: Self.buttons())))
        #expect(model.hint == "⌘⌫ clears filters")
        #expect(model.ringed)
        model.apply(Self.query([]))
        #expect(model.hint == "/ to edit")
        #expect(!model.ringed)
    }

    @Test func leavingTheResultsForgetsTheQuery() {
        let model = SearchQueryModel(engine: Engine())
        model.apply(Self.query([Self.chip(3, "from:", "ada")]))

        #expect(model.apply(.focusLeaveResults) == .query)
        #expect(model.chips.isEmpty)
        #expect(model.buttons.isEmpty)
        #expect(model.words.isEmpty)
    }
}
