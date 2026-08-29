import AppKit
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
            notifications.start()
            notifications.open = { [weak self] mailbox, message in
                self?.requested = (mailbox, message)
                self?.requestedToken += 1
                self?.open(mailbox: mailbox)
            }
            consumeEvents(from: session)
            // The platform observes and the engine is told. Callbacks arrive
            // on a background queue and may repeat the same answer; the
            // boundary absorbs that, nudging a reconnect only on a real
            // transition back, so there is nothing to debounce here.
            reachability.start { [weak self] offline in
                Task { @MainActor in self?.session?.setOffline(offline) }
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

    /// The folder the list currently has open, for deciding what is news.
    private(set) var showingMailbox: Int64?

    /// The message a notification click asked for, for the shell to open.
    ///
    /// A tuple rather than a struct because nothing else reads it, and paired
    /// with a counter because two clicks on the same notification are two
    /// requests: SwiftUI's `onChange` compares values, and the second would
    /// otherwise look like nothing happened.
    private(set) var requested: (mailbox: Int64, message: Int64?)?
    private(set) var requestedToken = 0

    private let notifications = MailNotifications()
    private let reachability = Reachability()

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
        showingMailbox = mailbox
        // Choosing a folder ends the search. Leaving results in the list under
        // a folder the sidebar now shows as selected would be the list saying
        // one thing and the sidebar another.
        query = ""
        session.openScope(.mailbox(mailbox: mailbox))
        controller.tableView?.reloadData()
    }

    /// What is currently typed in the search field. Empty means not searching.
    private(set) var query: String = ""

    /// Whether the list is showing results rather than a folder.
    var isSearching: Bool { !query.isEmpty }

    /// Run `query`, or restore the folder when it is empty.
    ///
    /// Called on every keystroke, and that is affordable because the index is
    /// local: `PRODUCT.md` budgets local search under 100 ms, and the boundary
    /// asserts it. If this ever has to debounce, the fix is the query being
    /// slow rather than the typing being fast.
    func search(_ query: String) {
        guard let session, case let .open(controller) = state else { return }
        self.query = query
        // Empty is "not searching", not "search for nothing" -- the second
        // would answer every message in the store and call it a result.
        if query.isEmpty {
            session.clearSearch()
        } else {
            session.search(query)
        }
        controller.tableView?.reloadData()
    }

    /// How many rows the open scope has, or zero when there is no session.
    var rowCount: UInt32 { session?.rowCount ?? 0 }

    /// Drain the engine's events for as long as the session is open.
    ///
    /// `nextEvent` is an `async fn` on the Rust side, so this is the same
    /// shape the GTK frontend uses — `glib::spawn_future_local` around
    /// `EventStream::next()` — rather than a polling timer. The task ends when
    /// `nextEvent` answers `nil`, which is what `shutdown` makes it do.
    private func consumeEvents(from session: PostioSession) {
        Task { @MainActor [weak self] in
            while let event = await session.nextEvent() {
                self?.handle(event)
            }
        }
    }

    /// React to one engine event.
    ///
    /// The `default:` arm is deliberate and ADR 0019 Q7 asks for it: the event
    /// union is append-only and one-way, so an application built against an
    /// older boundary has to degrade to ignoring a variant it does not know
    /// rather than failing to compile or crashing on it.
    private func handle(_ event: UiEvent) {
        guard case let .open(controller) = state else { return }
        switch event {
        case .pageReady:
            // The page the table asked for arrived. Redrawing everything is
            // right at this size and wrong at scale; narrowing it to the rows
            // that changed is what `reloadData(forRowIndexes:)` is for and
            // belongs with the rest of the list work.
            controller.tableView?.reloadData()
        case .messageListChanged, .messagesChanged, .messagesRemoved:
            controller.tableView?.reloadData()
        case .mailboxesChanged:
            mailboxes = session?.mailboxes ?? []
        case let .newMail(account, mailbox, messages):
            arrived(MailArrival(account: account, mailbox: mailbox, messages: messages))
        default:
            // Everything else is something this build has no opinion about.
            break
        }
    }

    /// Decide what to do about new mail, and do it.
    private func arrived(_ arrival: MailArrival) {
        let decision = MailNotifier.decide(
            arrival,
            showing: showingMailbox,
            // Asked at the moment the decision is made rather than tracked:
            // `isActive` is a live property of the application, and a cached
            // copy would go stale in exactly the window that matters.
            isActive: NSApplication.shared.isActive,
            mailboxName: mailboxes.first { $0.id == arrival.mailbox }?.name
        )
        guard case let .deliver(notification) = decision else { return }
        notifications.post(notification)
    }

    /// Stop the engines and drop the store, in that order.
    ///
    /// Not a `deinit`: that is nonisolated and cannot touch main-actor state.
    /// It has to be called from the application's termination handler, and it
    /// matters more than it looks — `postio-app` calls the equivalent before
    /// returning because the store is SQLCipher, and dropping an engine at
    /// process exit is exactly when libcrypto goes away underneath a thread
    /// still encrypting a page.
    func shutdown() {
        reachability.stop()
        session?.shutdown()
        session = nil
    }
}
