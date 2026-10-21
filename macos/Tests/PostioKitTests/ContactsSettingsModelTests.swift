import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// Settings > Contacts (specs/011-focus-contacts D7): the pane's model. Every
/// book here is a fake in memory: no test constructs `AppleAddressBook`, and
/// none can reach a person's real Contacts. The properties that matter are
/// that nothing is asked of the Mac until the person turns sync on, that a
/// refusal reads nothing, that a click on a switch only asks, and that there
/// is no write at all.
@MainActor
struct ContactsSettingsModelTests {
    final class Engine: ContactsSettingsEngine {
        var panes: [(SyncScopeFfi, SyncModeFfi)] = []
        var planned: [(Int, SyncScopeFfi, SyncModeFfi)] = []
        var clicks: [UInt32] = []
        var opened = 0
        var settings: SuggestionsViewFfi?

        func contactsPane(
            _ scope: SyncScopeFfi, _ mode: SyncModeFfi, _ status: SyncStatusFfi, _ plan: SyncPlanFfi?
        ) -> ContactsPaneFfi {
            panes.append((scope, mode))
            return ContactsPaneFfi(introLead: "Lead,", introRest: " rest", sync: nil)
        }

        func contactsSyncPlan(
            _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi
        ) -> SyncResultFfi {
            planned.append((records.count, scope, mode))
            return SyncResultFfi(
                sheet: SyncSheetFfi(
                    title: "Dry run", lines: [], account: nil, scopes: [], scopeNote: nil,
                    plan: SyncPlanFfi(
                        summary: "Would add 1, update 0, ask about 0, delete 0", lines: ["Add Grace to Contacts"],
                        foot: "Read only until writes are reviewed."),
                    primary: EditButtonFfi(label: "Done", key: nil), secondary: nil),
                error: nil)
        }

        func focusContactsSettingsOpened() { opened += 1 }
        func focusContactsSuggestion(_ index: UInt32) { clicks.append(index) }
        func focusContactsSettings() -> SuggestionsViewFfi? { settings }
    }

    func record(_ id: String) -> ExternalRecordFfi {
        ExternalRecordFfi(
            id: id, given: "Ada", family: "", emails: ["ada@example.org"], phones: [], organization: "",
            title: "")
    }

    @Test func buildingThePaneAsksNothingOfTheMacAndConstructsNoBook() {
        var made = 0
        let engine = Engine()
        let model = ContactsSettingsModel(engine: engine, makeBook: {
            made += 1
            return FakeSyncAddressBook()
        })
        model.opened()
        #expect(model.pane?.introLead == "Lead,")
        #expect(made == 0, "the book is not even made until sync is turned on")
        #expect(engine.planned.isEmpty)
        #expect(engine.opened == 1, "the switches are read")
    }

    @Test func aClickOnASwitchOnlyAsksAndTheViewComesFromTheEvent() {
        let engine = Engine()
        let model = ContactsSettingsModel(engine: engine, makeBook: { FakeSyncAddressBook() })
        model.toggle(2)
        #expect(engine.clicks == [2])
        #expect(model.suggestions == nil, "nothing is drawn as changed before Rust says so")
        let view = SuggestionsViewFfi(
            heading: "Suggestions",
            rows: [SuggestionRowFfi(title: "t", detail: "d", on: true)], footer: "f", error: nil)
        #expect(model.apply(.focusContactsSettings(view: view)))
        #expect(model.suggestions == view)
    }

    @Test func turningItOnAsksOnceReadsOneSnapshotAndShowsTheDryRun() async {
        let book = FakeSyncAddressBook(access: .notDetermined, grant: .authorized, records: [record("x1")])
        let engine = Engine()
        let model = ContactsSettingsModel(engine: engine, makeBook: { book })
        await model.preview()
        #expect(book.prompts == 1)
        #expect(book.snapshots == 1)
        #expect(engine.planned.count == 1 && engine.planned[0].0 == 1)
        #expect(model.plan?.summary == "Would add 1, update 0, ask about 0, delete 0")
        #expect(model.status == .previewed(summary: "Would add 1, update 0, ask about 0, delete 0"))
    }

    @Test func anAlreadyAuthorizedBookIsNotAskedAgain() async {
        let book = FakeSyncAddressBook(access: .authorized, records: [record("x1")])
        let model = ContactsSettingsModel(engine: Engine(), makeBook: { book })
        await model.preview()
        await model.preview()
        #expect(book.prompts == 0 && book.snapshots == 2)
    }

    @Test func aRefusalReadsNothingAndSaysSo() async {
        for access in [SyncAccess.notDetermined, .denied] {
            let book = FakeSyncAddressBook(access: access, grant: .denied, records: [record("x1")])
            let engine = Engine()
            let model = ContactsSettingsModel(engine: engine, makeBook: { book })
            await model.preview()
            #expect(book.snapshots == 0, "\(access): nothing was read")
            #expect(engine.planned.isEmpty)
            #expect(model.status == .declined)
            #expect(model.plan == nil)
        }
    }

    @Test func theScopeAndModeAreChosenBeforeAndReachThePlanAndAChoiceDropsAStalePlan() async {
        let book = FakeSyncAddressBook(access: .authorized, records: [record("x1")])
        let engine = Engine()
        let model = ContactsSettingsModel(engine: engine, makeBook: { book })
        model.choose(SyncScopeFfi.people)
        model.choose(SyncModeFfi.twoWay)
        await model.preview()
        #expect(engine.planned[0].1 == .people && engine.planned[0].2 == .twoWay)
        #expect(model.plan != nil)
        model.choose(SyncScopeFfi.everything)
        #expect(model.plan == nil && model.status == .notOn, "a plan for another scope is not the plan")
    }

    @Test func theRealStoreRefusesToRunUnderATestAndInADemo() {
        #expect(AppleAddressBook.refusesHere, "a test run must never reach anyone's Contacts")
    }
}
