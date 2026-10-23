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
        var brought: [(Int, SyncScopeFfi, SyncModeFfi, UInt64)] = []
        var bringResult = BringInResultFfi(note: "Brought 1 person into Postio. Nothing was written to Contacts.", error: nil)
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
                        summary: "Would bring in 1, add 0, update 0, ask about 0, delete 0",
                        lines: ["Bring Ada into Postio"],
                        foot: "Read only until writes are reviewed.",
                        bringIn: BringInFfi(label: "Bring 1 into Postio", fingerprint: 77)),
                    primary: EditButtonFfi(label: "Done", key: nil), secondary: nil),
                error: nil)
        }

        func contactsSyncBringIn(
            _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi, _ fingerprint: UInt64
        ) -> BringInResultFfi {
            brought.append((records.count, scope, mode, fingerprint))
            return bringResult
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
        #expect(model.plan?.summary == "Would bring in 1, add 0, update 0, ask about 0, delete 0")
        #expect(model.status == .previewed(summary: "Would bring in 1, add 0, update 0, ask about 0, delete 0"))
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

    @Test func bringingInSendsTheShownPlansFingerprintReadsTheBookAgainAndNeverAsksTwiceOrWrites() async {
        let book = FakeSyncAddressBook(access: .notDetermined, grant: .authorized, records: [record("x1")])
        let engine = Engine()
        let model = ContactsSettingsModel(engine: engine, makeBook: { book })
        await model.preview()
        #expect(model.plan?.bringIn?.label == "Bring 1 into Postio")
        model.bringIn()
        #expect(engine.brought.count == 1)
        #expect(engine.brought[0].0 == 1 && engine.brought[0].1 == .peopleAndCompanies)
        #expect(engine.brought[0].2 == .readOnly && engine.brought[0].3 == 77, "the plan that was shown")
        #expect(book.prompts == 1, "access was asked for once, by the preview")
        #expect(book.snapshots == 2)
        #expect(book.writes == 0, "bringing in changes Postio, not the address book")
        #expect(model.plan == nil, "the plan is spent; preview again for what is left")
        #expect(
            model.status == .broughtIn(note: "Brought 1 person into Postio. Nothing was written to Contacts."))
        #expect(model.error == nil)
    }

    @Test func aRefusedBringInKeepsNothingOfThePlanAndSaysWhy() async {
        let book = FakeSyncAddressBook(access: .authorized, records: [record("x1")])
        let engine = Engine()
        engine.bringResult = BringInResultFfi(
            note: nil, error: "Contacts changed since the preview. Preview again.")
        let model = ContactsSettingsModel(engine: engine, makeBook: { book })
        await model.preview()
        model.bringIn()
        #expect(model.error == "Contacts changed since the preview. Preview again.")
        #expect(model.plan == nil)
        #expect(book.writes == 0)
    }

    @Test func thereIsNothingToBringInBeforeAPreview() {
        let engine = Engine()
        let model = ContactsSettingsModel(engine: engine, makeBook: { FakeSyncAddressBook() })
        model.bringIn()
        #expect(engine.brought.isEmpty)
    }

    /// One store behind both the pane and the Contacts view, the way the
    /// engine has one: bringing people in changes it and says the people
    /// changed, and the view reads rows from it.
    final class Store: ContactsSettingsEngine {
        var people: [String] = ["Grace"]
        var pending: [UiEvent] = []
        func contactsPane(
            _ scope: SyncScopeFfi, _ mode: SyncModeFfi, _ status: SyncStatusFfi, _ plan: SyncPlanFfi?
        ) -> ContactsPaneFfi { ContactsPaneFfi(introLead: "", introRest: "", sync: nil) }
        func contactsSyncPlan(
            _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi
        ) -> SyncResultFfi {
            let fresh = records.filter { !people.contains($0.given) }
            return SyncResultFfi(
                sheet: SyncSheetFfi(
                    title: "Dry run", lines: [], account: nil, scopes: [], scopeNote: nil,
                    plan: SyncPlanFfi(
                        summary: "Would bring in \(fresh.count)", lines: fresh.map { "Bring \($0.given) into Postio" },
                        foot: "", bringIn: BringInFfi(label: "Bring \(fresh.count) into Postio", fingerprint: 5)),
                    primary: EditButtonFfi(label: "Done", key: nil), secondary: nil),
                error: nil)
        }
        func contactsSyncBringIn(
            _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi, _ fingerprint: UInt64
        ) -> BringInResultFfi {
            people += records.map(\.given).filter { !people.contains($0) }
            // What the engine's ContactsChanged makes the controller say.
            pending = [
                .focusContacts(view: ContactsModelTests.frame(total: UInt32(people.count))),
                .focusContactsRows(first: 0, count: 0),
            ]
            return BringInResultFfi(note: "Brought people into Postio.", error: nil)
        }
        func focusContactsSettingsOpened() {}
        func focusContactsSuggestion(_ index: UInt32) {}
        func focusContactsSettings() -> SuggestionsViewFfi? { nil }
    }

    final class StoreContacts: ContactsEngine {
        let store: Store
        init(_ store: Store) { self.store = store }
        func focusContactsRow(_ index: UInt32) -> ContactRowViewFfi? {
            guard Int(index) < store.people.count else { return nil }
            return ContactsModelTests.row(store.people[Int(index)])
        }
        func focusContactsTotal() -> UInt32 { UInt32(store.people.count) }
        func focusContactsCursor() -> UInt32? { 0 }
        func focusContactsCard() -> ContactCardViewFfi? { nil }
        func focusContactsTab(_ tab: ContactTabFfi) {}
        func focusContactsFilter(_ text: String) {}
        func focusContactsPoint(_ index: UInt32) {}
        func focusContactMergeKeep(_ index: UInt32) {}
        func focusContactSplit(_ address: String) {}
        func focusContactEdit() -> ContactEditViewFfi? { nil }
        func focusContactEditChange(_ change: EditChangeFfi) {}
        func focusContactCardSwitch(_ index: UInt32) {}
        func invoke(_ id: String) {}
    }

    @Test func thePeopleBroughtInAppearInTheContactsViewAndTheBookIsOnlyRead() async {
        let store = Store()
        let book = FakeSyncAddressBook(
            access: .notDetermined, grant: .authorized,
            records: [record("x1"), record("x2")].enumerated().map { index, record in
                ExternalRecordFfi(
                    id: record.id, given: index == 0 ? "Ada" : "Ben", family: "", emails: record.emails,
                    phones: [], organization: "", title: "")
            })
        let settings = ContactsSettingsModel(engine: store, makeBook: { book })
        let contacts = ContactsModel(engine: StoreContacts(store))
        _ = contacts.apply(.focusContacts(view: ContactsModelTests.frame(total: 1)))
        #expect(contacts.row(at: 1) == nil, "only Grace is there")

        await settings.preview()
        settings.bringIn()
        for event in store.pending { _ = contacts.apply(event) }

        #expect(contacts.total == 3)
        #expect(contacts.row(at: 1)?.name == "Ada")
        #expect(contacts.row(at: 2)?.name == "Ben")
        #expect(book.prompts == 1, "access was requested once")
        #expect(book.writes == 0, "the address book received nothing")
    }
}
