import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The Contacts view's model (specs/011-focus-contacts S7, handoff screens 01
/// to 03). The controller says what the frame, the rows and the card are;
/// the Mac keeps none of it, asks for a row at a time and sends back a click
/// on a tab, a row, a button or the filter.
@MainActor
struct ContactsModelTests {
    final class Engine: ContactsEngine {
        var rows: [UInt32: ContactRowViewFfi] = [:]
        var asked: [UInt32] = []
        var tabs: [ContactTabFfi] = []
        var filters: [String] = []
        var points: [UInt32] = []
        var invoked: [String] = []
        var cursor: UInt32? = 0
        var card: ContactCardViewFfi?
        var edit: ContactEditViewFfi?
        var changes: [EditChangeFfi] = []
        var switches: [UInt32] = []
        func focusContactEdit() -> ContactEditViewFfi? { edit }
        func focusContactEditChange(_ change: EditChangeFfi) { changes.append(change) }
        func focusContactCardSwitch(_ index: UInt32) { switches.append(index) }
        func focusContactsRow(_ index: UInt32) -> ContactRowViewFfi? {
            asked.append(index)
            return rows[index]
        }
        func focusContactsTotal() -> UInt32 { UInt32(rows.count) }
        func focusContactsCursor() -> UInt32? { cursor }
        func focusContactsCard() -> ContactCardViewFfi? { card }
        func focusContactsTab(_ tab: ContactTabFfi) { tabs.append(tab) }
        func focusContactsFilter(_ text: String) { filters.append(text) }
        func focusContactsPoint(_ index: UInt32) { points.append(index) }
        func invoke(_ id: String) { invoked.append(id) }
    }

    nonisolated static func frame(total: UInt32 = 3, filter: String = "") -> ContactsViewFfi {
        ContactsViewFfi(
            strip: ContactsStripFfi(
                title: "Contacts", key: "g c", total: "3",
                tabs: [
                    ContactTabViewFfi(tab: .all, title: "All", count: nil, on: true),
                    ContactTabViewFfi(tab: .people, title: "People", count: "3", on: false),
                ],
                filterPlaceholder: "Filter contacts", filterKey: "/", newLabel: "New contact",
                newKey: "mod+shift+n", newEnabled: false, review: nil),
            footer: ContactsFooterFfi(hints: [], status: "3 contacts · 1 saved"),
            total: total, filter: filter, loaded: true)
    }

    nonisolated static func row(_ name: String, letter: String? = nil) -> ContactRowViewFfi {
        ContactRowViewFfi(
            id: 1, avatar: AvatarFfi(text: "AM", rgb: 0xb0624a, round: true), name: name, tag: nil,
            address: "ada@example.org", when: "today", countLine: "431 messages", letter: letter)
    }

    nonisolated static func button(enabled: Bool) -> CardButtonFfi {
        CardButtonFfi(label: "Write", command: "contact_compose", key: "c", primary: true, enabled: enabled)
    }

    @Test func itOpensOnAFrameRedrawsAndClosesOnTheControllersWord() {
        let model = ContactsModel(engine: Engine())
        #expect(!model.isOpen)
        #expect(model.apply(.focusContacts(view: Self.frame())) == .open)
        #expect(model.total == 3)
        #expect(model.apply(.focusContacts(view: Self.frame(total: 2))) == .redraw)
        #expect(model.apply(.focusContactsCursor(index: 1)) == .redraw)
        #expect(model.cursor == 1)
        #expect(model.apply(.focusContactsRows(first: 0, count: 3)) == .redraw)
        #expect(model.apply(.focusCloseSurface(kind: .contacts)) == .close)
        #expect(!model.isOpen)
        #expect(model.apply(.focusCloseSurface(kind: .contacts)) == nil, "closed once")
    }

    @Test func aRowIsAskedForAtAnIndexAndRedrawnWhenItsPageLands() {
        let engine = Engine()
        let model = ContactsModel(engine: engine)
        model.apply(.focusContacts(view: Self.frame()))
        #expect(model.row(at: 2) == nil)
        #expect(engine.asked == [2], "a miss asks the engine, which asks for the page")
        engine.rows[2] = Self.row("Ben Adeyemi", letter: "B")
        let before = model.revision
        model.apply(.focusContactsRows(first: 0, count: 3))
        #expect(model.revision == before + 1)
        #expect(model.row(at: 2)?.letter == "B")
    }

    @Test func aClickOnATabARowTheFilterAndAButtonIsSentAndAnUnreadyOneIsNot() {
        let engine = Engine()
        let model = ContactsModel(engine: engine)
        model.choose(.people)
        model.point(1)
        model.typed("ada")
        #expect(engine.tabs.isEmpty && engine.points.isEmpty && engine.filters.isEmpty, "nothing while closed")
        model.apply(.focusContacts(view: Self.frame()))
        model.choose(.people)
        model.point(1)
        model.point(9)
        model.typed("ada")
        model.run(Self.button(enabled: true))
        model.run(Self.button(enabled: false))
        #expect(engine.tabs == [.people])
        #expect(engine.points == [1], "a row past the end is not a click")
        #expect(engine.filters == ["ada"])
        #expect(engine.invoked == ["contact_compose"])
    }

    @Test func theFilterFieldFollowsTheControllersAndEscapeAsksForTheKeyboard() {
        let model = ContactsModel(engine: Engine())
        model.apply(.focusContacts(view: Self.frame(filter: "ben")))
        #expect(model.filter == "ben")
        model.apply(.focusContacts(view: Self.frame(filter: "")))
        #expect(model.filter == "", "Escape in the controller cleared it")
        let asked = model.filterRequest
        model.apply(.focusContactsFocusFilter)
        #expect(model.filterRequest == asked + 1)
    }

    // MARK: the Edit contact form (specs/011-focus-contacts S8)

    nonisolated static func form(given: String = "Ada", armed: String? = nil) -> ContactEditViewFfi {
        ContactEditViewFfi(
            title: "Editing Ada Moreno", note: "saved in Postio",
            cancel: EditButtonFfi(label: "Cancel", key: "Escape"),
            save: EditButtonFfi(label: "Save", key: "cmd+s"),
            avatar: AvatarFfi(text: "AM", rgb: 0xb0624a, round: true), photoNote: "Initials",
            kinds: [EditSegmentFfi(label: "Person", on: true)], kindNote: "Detected",
            given: given, family: "Moreno",
            showAs: [EditSegmentFfi(label: "Ada Moreno", on: true)], showNote: "Inbox",
            workTitle: "", workCompany: "",
            addresses: [
                EditAddressFfi(text: "ada@example.org", label: "Work", primary: true, bounced: false)
            ],
            addAddress: "+ Add address", addressNote: "Primary", seen: [], groups: [],
            addGroup: "+ Add to group", groupPicker: nil, groupNote: "Groups", problem: nil, armed: armed, hints: [],
            focus: .given)
    }

    @Test func theFormOpensOnTheControllersWordAndAChangeIsReadBackAtOnce() {
        let engine = Engine()
        let model = ContactsModel(engine: engine)
        model.apply(.focusContacts(view: Self.frame()))
        #expect(model.edit == nil)
        #expect(model.apply(.focusContactEdit(view: Self.form())) == .redraw)
        #expect(model.edit?.given == "Ada")

        engine.edit = Self.form(given: "Adele")
        model.change(.given(text: "Adele"))
        #expect(engine.changes == [.given(text: "Adele")])
        #expect(model.edit?.given == "Adele", "read back, not waited for")

        model.apply(.focusContactEdit(view: nil))
        #expect(model.edit == nil)
    }

    @Test func saveAndCancelSendTheKeysAndNothingIsSentWithoutAForm() {
        let engine = Engine()
        let model = ContactsModel(engine: engine)
        model.apply(.focusContacts(view: Self.frame()))
        model.save()
        model.cancel()
        model.change(.addAddress)
        #expect(engine.invoked.isEmpty && engine.changes.isEmpty, "no form, no command")
        model.apply(.focusContactEdit(view: Self.form()))
        model.save()
        engine.edit = Self.form(armed: "Press Esc again to discard your changes.")
        model.cancel()
        #expect(engine.invoked == ["contact_save", "back"])
        #expect(model.edit?.armed != nil, "an armed Esc is the controller's, drawn")
    }

    @Test func closingTheViewPutsTheFormAwayAndNewContactSendsItsKey() {
        let engine = Engine()
        let model = ContactsModel(engine: engine)
        model.startNew()
        #expect(engine.invoked.isEmpty, "closed")
        model.apply(.focusContacts(view: Self.frame()))
        model.startNew()
        #expect(engine.invoked == ["contact_new"])
        model.apply(.focusContactEdit(view: Self.form()))
        model.startNew()
        #expect(engine.invoked == ["contact_new"], "not over a form already open")
        model.apply(.focusCloseSurface(kind: .contacts))
        #expect(model.edit == nil)
    }
}
