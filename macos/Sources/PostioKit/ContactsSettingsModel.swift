import Foundation
import Observation
import PostioFFI

/// Whether the person has let Postio into an address book.
public enum SyncAccess: Equatable, Sendable {
    case notDetermined
    case denied
    case authorized
}

/// The address book behind sync (specs/011-focus-contacts S12): a protocol, so
/// every test, the demo and every storyboard use a fake, and the real store
/// (`AppleAddressBook`) is one conformance nothing here constructs until a
/// person turns sync on.
///
/// It has a snapshot and no write: this side of the boundary only moves bytes
/// *out* of an address book. There is no `apply`, so there is no write path to
/// guard; when one exists it will be behind the build flag and the person's
/// confirmation of the dry-run plan (`postio_contact_sync::write`), and not
/// before the maintainer has reviewed what a dry run printed. Bringing people
/// *into Postio* is not that path: it hands the engine a snapshot and writes
/// Postio's own database.
@MainActor
public protocol SyncAddressBook: AnyObject {
    /// What the person has said. Never prompts.
    var access: SyncAccess { get }
    /// The system's prompt. Called only by `ContactsSettingsModel.preview`.
    func requestAccess() async -> SyncAccess
    /// Every record, for the engine's plan.
    func snapshot() throws -> [ExternalRecordFfi]
}

/// What Settings > Contacts asks of the engine.
public protocol ContactsSettingsEngine: AnyObject {
    func contactsPane(
        _ scope: SyncScopeFfi, _ mode: SyncModeFfi, _ status: SyncStatusFfi, _ plan: SyncPlanFfi?
    ) -> ContactsPaneFfi
    func contactsSyncPlan(
        _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi
    ) -> SyncResultFfi
    /// Applies the Postio side of the plan with this fingerprint. Postio's
    /// database only.
    func contactsSyncBringIn(
        _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi,
        _ fingerprint: UInt64
    ) -> BringInResultFfi
    func focusContactsSettingsOpened()
    func focusContactsSuggestion(_ index: UInt32)
    func focusContactsSettings() -> SuggestionsViewFfi?
}

extension PostioSession: ContactsSettingsEngine {}

/// Settings > Contacts (specs/011-focus-contacts D7, handoff screen 08): the
/// pane's words, the four Suggestions switches, and the sync dry run. Rust
/// decides and says everything; this asks the Mac for access when, and only
/// when, the person presses "Turn on and preview", reads one snapshot, and
/// sends a click on a switch. The preview writes to neither store; "Bring in"
/// writes Postio's, on the person's click, and never the address book.
@MainActor
@Observable
public final class ContactsSettingsModel {
    /// What syncs.
    public private(set) var scope: SyncScopeFfi = .peopleAndCompanies
    /// How a sync would treat the two sides.
    public private(set) var mode: SyncModeFfi = .readOnly
    /// How far the last attempt got.
    public private(set) var status: SyncStatusFfi = .notOn
    /// The last dry run.
    public private(set) var plan: SyncPlanFfi?
    /// Why the last attempt did nothing, in words for the screen.
    public private(set) var error: String?
    /// The pane's static words and the sync section.
    public private(set) var pane: ContactsPaneFfi?
    /// The Suggestions section, once the host has said where the switches are.
    public private(set) var suggestions: SuggestionsViewFfi?

    @ObservationIgnored private let engine: ContactsSettingsEngine
    @ObservationIgnored private let makeBook: () -> SyncAddressBook
    @ObservationIgnored private var book: SyncAddressBook?

    /// `makeBook` is called once, by `preview`, and never before: a real
    /// address book is not even constructed until the person asks.
    public init(engine: ContactsSettingsEngine, makeBook: @escaping () -> SyncAddressBook) {
        self.engine = engine
        self.makeBook = makeBook
        refresh()
    }

    private func refresh() {
        pane = engine.contactsPane(scope, mode, status, plan)
    }

    /// The pane appeared: read the switches. Asks nothing of the Mac.
    public func opened() {
        error = nil
        suggestions = engine.focusContactsSettings()
        engine.focusContactsSettingsOpened()
    }

    /// A click on the Suggestions switch at `index`.
    public func toggle(_ index: Int) {
        engine.focusContactsSuggestion(UInt32(index))
    }

    /// Apply `event` if it is this pane's.
    @discardableResult
    public func apply(_ event: UiEvent) -> Bool {
        guard case let .focusContactsSettings(view) = event else { return false }
        suggestions = view
        return true
    }

    /// A click on a choice of "What syncs". A plan made for another scope is
    /// no longer the plan, so it goes.
    public func choose(_ next: SyncScopeFfi) {
        guard next != scope else { return }
        scope = next
        forgetPlan()
    }

    /// A click on Two-way or Read only.
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

    /// Turn on and preview: the one place access is asked for, and only if
    /// the system has not already decided; then one snapshot, and the
    /// engine's dry run of it.
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

    /// "Bring N into Postio": the person's yes to the plan shown. Reads one
    /// more snapshot (access was granted by the preview and is not asked
    /// again) and hands it to the engine with the shown plan's fingerprint; a
    /// book that changed since is refused there and needs a new preview. The
    /// address book is read, never written.
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

/// An address book in memory that counts what reaches it: the demo's, and
/// every test's.
@MainActor
public final class FakeSyncAddressBook: SyncAddressBook {
    public private(set) var access: SyncAccess
    private let grant: SyncAccess
    private let records: [ExternalRecordFfi]
    public private(set) var prompts = 0
    public private(set) var snapshots = 0
    /// Writes that reached the book: always 0, because the protocol has no
    /// write to call. Here so a test says the property out loud.
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
