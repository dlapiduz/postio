import PostioFFI
import PostioKit
import SwiftUI

/// The engine, and what to show when it will not start.
///
/// Opening a session reads the local store's key from the login Keychain
/// (ADR 0014), and an ad-hoc-signed build has a new code identity on every
/// rebuild — so macOS asks again after each one. That is a real state the
/// application has to render rather than crash through, and it is the reason
/// this holds a *result* rather than a session.
// `@MainActor` because everything it holds is: the table controller, the
// web view's handlers, and the views that read it. The engine's own work
// happens on its runtime, not here.
@MainActor
@Observable
final class Engine {
    enum State {
        /// A live session, with the list driven from it.
        case open(MessageTableController)
        /// No session, and the sentence explaining why.
        case unavailable(String)
    }

    private(set) var state: State
    private(set) var session: PostioSession?

    init() {
        do {
            let session = try PostioSession.open()
            self.session = session
            state = .open(MessageTableController(source: SessionRowSource(session: session)))
            // Nothing was ever fetched before this: the store opened and
            // stayed empty because no engine had been started (#648).
            mailboxes = session.mailboxes
            if let started = try? session.startSyncing(), started > 0 {
                // Engines run on their own runtime; the list repaints from
                // events rather than from anything awaited here.
            }
        } catch {
            // The message the boundary wrote, not one invented here: a locked
            // keychain says how to unlock it, and a broken store says what
            // broke. Replacing either with "could not start" throws away the
            // only instruction the user gets.
            state = .unavailable(String(describing: error))
            session = nil
        }
    }

    /// Every folder, flat, with parent ids.
    private(set) var mailboxes: [MailboxFfi] = []

    /// Folders with no parent, in the order the store returned them.
    var folderRoots: [MailboxFfi] {
        mailboxes.filter { $0.parent == nil }
    }

    /// The children of `parent`.
    func children(of parent: Int64) -> [MailboxFfi] {
        mailboxes.filter { $0.parent == parent }
    }

    /// Show a folder's messages.
    ///
    /// Re-scoping drops the selection on the other side, which is right:
    /// "these twelve" means something else the moment the list does, and an
    /// action carrying a selection across would land on mail the user cannot
    /// see.
    func open(mailbox: Int64) {
        guard let session, case let .open(controller) = state else { return }
        session.openScope(.mailbox(mailbox: mailbox))
        controller.tableView?.reloadData()
    }

    /// How many rows the open scope has, or zero when there is no session.
    var rowCount: UInt32 { session?.rowCount ?? 0 }

    /// Stop the engines and drop the store, in that order.
    ///
    /// Not a `deinit`: that is nonisolated and cannot touch main-actor state.
    /// It has to be called from the application's termination handler, and it
    /// matters more than it looks — `postio-app` calls the equivalent before
    /// returning because the store is SQLCipher, and dropping an engine at
    /// process exit is exactly when libcrypto goes away underneath a thread
    /// still encrypting a page.
    func shutdown() {
        session?.shutdown()
        session = nil
    }
}
