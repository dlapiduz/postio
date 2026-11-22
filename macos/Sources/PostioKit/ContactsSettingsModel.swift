import Foundation
import Observation
import PostioFFI

public enum SyncAccess: Equatable, Sendable {
    case notDetermined
    case denied
    case authorized
}

/// A protocol so every test, the demo and every storyboard use a fake; the real store
/// (`AppleAddressBook`) is constructed only when a person turns sync on. It has a snapshot and no
/// write: any write path must sit behind the build flag and the person's confirmation of the
/// dry-run plan (`postio_contact_sync::write`).
@MainActor
public protocol SyncAddressBook: AnyObject {
    var access: SyncAccess { get }
    func requestAccess() async -> SyncAccess
    func snapshot() throws -> [ExternalRecordFfi]
}

public protocol ContactsSettingsEngine: AnyObject {
    func contactsPane(
        _ scope: SyncScopeFfi, _ mode: SyncModeFfi, _ status: SyncStatusFfi, _ plan: SyncPlanFfi?
    ) -> ContactsPaneFfi
    func contactsSyncPlan(
        _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi
    ) -> SyncResultFfi
    func contactsSyncBringIn(
        _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi,
        _ fingerprint: UInt64
    ) -> BringInResultFfi
    func focusContactsSettingsOpened()
    func focusContactsSuggestion(_ index: UInt32)
    func focusContactsSettings() -> SuggestionsViewFfi?
}

extension PostioSession: ContactsSettingsEngine {}

/// Asks the Mac for access only when the person presses "Turn on and preview". The preview writes
/// to neither store; "Bring in" writes Postio's, on the person's click, never the address book.
@MainActor
@Observable
public final class ContactsSettingsModel {
    public private(set) var scope: SyncScopeFfi = .peopleAndCompanies
    public private(set) var mode: SyncModeFfi = .readOnly
    public private(set) var status: SyncStatusFfi = .notOn
    public private(set) var plan: SyncPlanFfi?
    public private(set) var error: String?
    public private(set) var pane: ContactsPaneFfi?
    public private(set) var suggestions: SuggestionsViewFfi?

    @ObservationIgnored private let engine: ContactsSettingsEngine
    @ObservationIgnored private let makeBook: () -> SyncAddressBook
    @ObservationIgnored private var book: SyncAddressBook?

    /// `makeBook` is called once, by `preview`, and never before: a real address book is not
    /// constructed until the person asks.
    public init(engine: ContactsSettingsEngine, makeBook: @escaping () -> SyncAddressBook) {
        self.engine = engine
        self.makeBook = makeBook
        refresh()
    }

    private func refresh() {
        pane = engine.contactsPane(scope, mode, status, plan)
    }

    public func opened() {
        error = nil
        suggestions = engine.focusContactsSettings()
        engine.focusContactsSettingsOpened()
    }

    public func toggle(_ index: Int) {
        engine.focusContactsSuggestion(UInt32(index))
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Bool {
        guard case let .focusContactsSettings(view) = event else { return false }
        suggestions = view
        return true
    }

    public func choose(_ next: SyncScopeFfi) {
        guard next != scope else { return }
        scope = next
        forgetPlan()
    }

    public func choose(_ next: SyncModeFfi) {
        guard next != mode else { return }
        mode = next
        forgetPlan()
    }

    private func forgetPlan() {
        plan = nil
        status = .notOn
        error = nil
        refresh()
    }

    public func preview() async {
        let book = self.book ?? makeBook()
        self.book = book
        var access = book.access
        if access == .notDetermined { access = await book.requestAccess() }
        guard access == .authorized else {
            plan = nil
            status = .declined
            refresh()
            return
        }
        do {
            let result = engine.contactsSyncPlan(try book.snapshot(), scope, mode)
            error = result.error
            if let shown = result.sheet?.plan {
                plan = shown
                status = .previewed(summary: shown.summary)
            }
        } catch {
            self.error = "Postio could not read Contacts."
        }
        refresh()
    }

    /// Reads one more snapshot and hands it to the engine with the shown plan's fingerprint; a book
    /// that changed since is refused there and needs a new preview.
    public func bringIn() {
        guard let book, let bring = plan?.bringIn else { return }
        guard book.access == .authorized else {
            plan = nil
            status = .declined
            refresh()
            return
        }
        do {
            let result = engine.contactsSyncBringIn(try book.snapshot(), scope, mode, bring.fingerprint)
            if let note = result.note {
                error = nil
                plan = nil
                status = .broughtIn(note: note)
            } else {
                error = result.error
                if result.error != nil { plan = nil }
            }
        } catch {
            self.error = "Postio could not read Contacts."
        }
        refresh()
    }
}

@MainActor
public final class FakeSyncAddressBook: SyncAddressBook {
    public private(set) var access: SyncAccess
    private let grant: SyncAccess
    private let records: [ExternalRecordFfi]
    public private(set) var prompts = 0
    public private(set) var snapshots = 0
    public private(set) var writes = 0

    public init(
        access: SyncAccess = .authorized, grant: SyncAccess = .denied,
        records: [ExternalRecordFfi] = []
    ) {
        self.access = access
        self.grant = grant
        self.records = records
    }

    public func requestAccess() async -> SyncAccess {
        prompts += 1
        access = grant
        return access
    }

    public func snapshot() throws -> [ExternalRecordFfi] {
        snapshots += 1
        return records
    }
}
