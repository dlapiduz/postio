import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// Sync with Contacts, as a dry run (specs/011-focus-contacts S12). Every
/// book here is a fake in memory: no test constructs `AppleAddressBook`, and
/// none can reach a person's real Contacts. The properties that matter are
/// that nothing is asked of the Mac until the person turns sync on, that a
/// refusal reads nothing, and that there is no write at all.
@MainActor
struct ContactsSyncModelTests {
    final class Engine: ContactsSyncEngine {
        var intros: [SyncScopeFfi] = []
        var planned: [(Int, SyncScopeFfi)] = []
        var denied = 0
        func contactsSyncIntro(_ scope: SyncScopeFfi) -> SyncSheetFfi {
            intros.append(scope)
            return ContactsSyncModelTests.intro(scope)
        }
        func contactsSyncDenied() -> SyncSheetFfi {
            denied += 1
            return ContactsSyncModelTests.refusal()
        }
        func contactsSyncPlan(_ records: [ExternalRecordFfi], _ scope: SyncScopeFfi) -> SyncResultFfi {
            planned.append((records.count, scope))
            return SyncResultFfi(sheet: ContactsSyncModelTests.plan(), error: nil)
        }
    }

    nonisolated static func button(_ label: String, _ key: String?) -> EditButtonFfi {
        EditButtonFfi(label: label, key: key)
    }

    nonisolated static func intro(_ scope: SyncScopeFfi) -> SyncSheetFfi {
        SyncSheetFfi(
            title: "Sync with Contacts", lines: ["Your contacts live in Postio."],
            account: SyncAccountFfi(
                title: "Contacts on this Mac (iCloud)", mode: "Read only",
                note: "Read only until writes are reviewed"),
            scopes: [
                SyncScopeOptionFfi(label: "People", scope: .people, on: scope == .people),
                SyncScopeOptionFfi(
                    label: "People and companies", scope: .peopleAndCompanies,
                    on: scope == .peopleAndCompanies),
                SyncScopeOptionFfi(label: "Everything saved", scope: .everything, on: scope == .everything),
            ],
            scopeNote: "note", plan: nil, primary: button("Turn on and preview", "Return"),
            secondary: button("Not now", "Escape"))
    }

    nonisolated static func plan() -> SyncSheetFfi {
        SyncSheetFfi(
            title: "Dry run: nothing was written", lines: [], account: nil, scopes: [], scopeNote: nil,
            plan: SyncPlanFfi(
                summary: "Would add 1, update 0, ask about 0, delete 0", lines: ["Add Grace to Contacts"],
                foot: "Read only until writes are reviewed."),
            primary: button("Done", "Escape"), secondary: nil)
    }

    nonisolated static func refusal() -> SyncSheetFfi {
        SyncSheetFfi(
            title: "Contacts access was declined", lines: ["Postio read nothing."], account: nil,
            scopes: [], scopeNote: nil, plan: nil, primary: button("Done", "Escape"), secondary: nil)
    }

    func record(_ id: String) -> ExternalRecordFfi {
        ExternalRecordFfi(
            id: id, given: "Ada", family: "", emails: ["ada@example.org"], phones: [], organization: "",
            title: "")
    }

    @Test func openingTheSheetAsksNothingOfTheMacAndConstructsNoBook() {
        var made = 0
        let engine = Engine()
        let model = ContactsSyncModel(engine: engine, makeBook: {
            made += 1
            return FakeSyncAddressBook()
        })
        model.open()
        #expect(model.sheet?.title == "Sync with Contacts")
        #expect(made == 0, "the book is not even made until sync is turned on")
        #expect(engine.planned.isEmpty)
    }

    @Test func turningItOnAsksOnceReadsOneSnapshotAndShowsTheDryRun() async {
        let book = FakeSyncAddressBook(access: .notDetermined, grant: .authorized, records: [record("x1")])
        let engine = Engine()
        let model = ContactsSyncModel(engine: engine, makeBook: { book })
        model.open()
        await model.turnOn()
        #expect(book.prompts == 1)
        #expect(book.snapshots == 1)
        #expect(engine.planned.count == 1 && engine.planned[0].0 == 1)
        #expect(model.sheet?.title == "Dry run: nothing was written")
        #expect(model.sheet?.plan?.summary == "Would add 1, update 0, ask about 0, delete 0")
    }

    @Test func anAlreadyAuthorizedBookIsNotAskedAgain() async {
        let book = FakeSyncAddressBook(access: .authorized, records: [record("x1")])
        let model = ContactsSyncModel(engine: Engine(), makeBook: { book })
        model.open()
        await model.turnOn()
        #expect(book.prompts == 0 && book.snapshots == 1)
    }

    @Test func aRefusalReadsNothingAndSaysSo() async {
        for access in [SyncAccess.notDetermined, .denied] {
            let book = FakeSyncAddressBook(access: access, grant: .denied, records: [record("x1")])
            let engine = Engine()
            let model = ContactsSyncModel(engine: engine, makeBook: { book })
            model.open()
            await model.turnOn()
            #expect(book.snapshots == 0, "\(access): nothing was read")
            #expect(engine.planned.isEmpty && engine.denied == 1)
            #expect(model.sheet?.title == "Contacts access was declined")
        }
    }

    @Test func theScopeIsChosenBeforeTurningOnAndReachesThePlan() async {
        let book = FakeSyncAddressBook(access: .authorized, records: [record("x1")])
        let engine = Engine()
        let model = ContactsSyncModel(engine: engine, makeBook: { book })
        model.open()
        model.choose(.people)
        #expect(model.scope == .people)
        await model.turnOn()
        #expect(engine.planned[0].1 == .people)
        // Once there is a plan, the scope is not a choice any more.
        model.choose(.everything)
        #expect(model.scope == .people)
    }

    @Test func returnOnTheIntroTurnsItOnAndOnThePlanIsDone() async {
        let book = FakeSyncAddressBook(access: .authorized, records: [record("x1")])
        let model = ContactsSyncModel(engine: Engine(), makeBook: { book })
        model.open()
        await model.primary()
        #expect(model.sheet?.plan != nil)
        await model.primary()
        #expect(model.sheet == nil)
    }

    @Test func notNowAndDoneJustCloseAndTurnOnWithoutASheetDoesNothing() async {
        let book = FakeSyncAddressBook(access: .notDetermined, grant: .authorized)
        let model = ContactsSyncModel(engine: Engine(), makeBook: { book })
        await model.turnOn()
        #expect(book.prompts == 0, "no sheet, no prompt")
        model.open()
        model.done()
        #expect(model.sheet == nil && book.prompts == 0)
    }

    @Test func theRealStoreRefusesToRunUnderATestAndInADemo() {
        #expect(AppleAddressBook.refusesHere, "a test run must never reach anyone's Contacts")
    }
}
