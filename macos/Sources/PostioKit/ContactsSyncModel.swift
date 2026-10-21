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
/// before the maintainer has reviewed what a dry run printed.
@MainActor
public protocol SyncAddressBook: AnyObject {
    /// What the person has said. Never prompts.
    var access: SyncAccess { get }
    /// The system's prompt. Called only by `ContactsSyncModel.turnOn`.
    func requestAccess() async -> SyncAccess
    /// Every record, for the engine's plan.
    func snapshot() throws -> [ExternalRecordFfi]
}

/// What the sync sheet asks of the engine.
public protocol ContactsSyncEngine: AnyObject {
    func contactsSyncIntro(_ scope: SyncScopeFfi) -> SyncSheetFfi
    func contactsSyncDenied() -> SyncSheetFfi
    func contactsSyncPlan(_ records: [ExternalRecordFfi], _ scope: SyncScopeFfi) -> SyncResultFfi
}

extension PostioSession: ContactsSyncEngine {}

/// File ▸ Sync Contacts: the sheet that explains it, and the dry run it ends
/// in. Rust decides and says everything (`SyncSheetFfi`); this asks the Mac
/// for access when, and only when, the person presses Turn on and preview, and
/// reads one snapshot. Nothing is written to either store.
@MainActor
@Observable
public final class ContactsSyncModel {
    /// The sheet over the window; `nil` while there is none.
    public private(set) var sheet: SyncSheetFfi?
    /// What syncs.
    public private(set) var scope: SyncScopeFfi = .peopleAndCompanies
    /// Why the last attempt did nothing, in words for the screen.
    public private(set) var error: String?

    @ObservationIgnored private let engine: ContactsSyncEngine
    @ObservationIgnored private let makeBook: () -> SyncAddressBook
    @ObservationIgnored private var book: SyncAddressBook?

    /// `makeBook` is called once, by `turnOn`, and never before: a real
    /// address book is not even constructed until the person asks.
    public init(engine: ContactsSyncEngine, makeBook: @escaping () -> SyncAddressBook) {
        self.engine = engine
        self.makeBook = makeBook
    }

    /// Whether the sheet is up.
    public var isOpen: Bool { sheet != nil }

    /// File ▸ Sync Contacts: the explainer. Asks nothing of the Mac.
    public func open() {
        error = nil
        sheet = engine.contactsSyncIntro(scope)
    }

    /// A click on a choice of "What syncs".
    public func choose(_ next: SyncScopeFfi) {
        guard sheet?.plan == nil, sheet?.account != nil else { return }
        scope = next
        sheet = engine.contactsSyncIntro(next)
    }

    /// Turn on and preview: the one place access is asked for, and only if
    /// the system has not already decided; then one snapshot, and the engine's
    /// dry run of it.
    public func turnOn() async {
        guard let shown = sheet, shown.account != nil else { return }
        let book = self.book ?? makeBook()
        self.book = book
        var access = book.access
        if access == .notDetermined { access = await book.requestAccess() }
        guard access == .authorized else {
            sheet = engine.contactsSyncDenied()
            return
        }
        do {
            let result = engine.contactsSyncPlan(try book.snapshot(), scope)
            error = result.error
            if let plan = result.sheet { sheet = plan }
        } catch {
            self.error = "Postio could not read Contacts."
        }
    }

    /// Return on the sheet: its main action.
    public func primary() async {
        if sheet?.account != nil {
            await turnOn()
        } else {
            done()
        }
    }

    /// Not now, Done, or Escape.
    public func done() {
        sheet = nil
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
