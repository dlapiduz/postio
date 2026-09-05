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
        /// The store is being opened, off this actor. See ``Engine/init()``.
        case opening
        /// A live session, with the list driven from it.
        case open(MessageTableController)
        /// No session, and the sentence explaining why.
        case unavailable(String)
    }

    private(set) var state: State = .opening
    private(set) var session: PostioSession?

    /// Starts the log, then opens the store **off this actor**.
    ///
    /// `Session::open` says not to call it on the main actor and means it: the
    /// store's key comes from the login Keychain, that round trip can wait on
    /// a user prompt, and `@State private var engine = Engine()` runs inside
    /// `App.init()` — before SwiftUI has a scene. Done synchronously, the
    /// application appeared in the Dock and drew no window at all, parked in
    /// `store_key_blocking` while macOS asked a question about an application
    /// that was not on screen to be asked about (#1146).
    ///
    /// An ad-hoc-signed build has a new code identity on every rebuild, so the
    /// Keychain asks again after each one — this is the ordinary path here,
    /// not an edge case.
    ///
    /// So opening is a *state*, and the window that draws it is also what the
    /// Keychain prompt has to appear in front of.
    init() {
        // First, so the two things most likely to fail on this platform -- the
        // Keychain refusing and the store refusing to migrate -- say so
        // somewhere rather than arriving as an empty window.
        PostioSession.startLogging()
        Task.detached(priority: .userInitiated) {
            // `PostioSession` is `@unchecked Sendable` and this is the call
            // that must not run on the main actor, so it happens here and only
            // its *result* hops back.
            let opened = Result { try PostioSession.open() }
            await MainActor.run { [weak self] in self?.adopt(opened) }
        }
    }

    /// Take up a session that opened, or record why one did not.
    ///
    /// Everything that needs a session is wired here rather than in `init`,
    /// because until this runs there is not one to wire anything to.
    private func adopt(_ opened: Result<PostioSession, Error>) {
        switch opened {
        case let .success(session):
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
            // Keystrokes, resolved by the core (#656). Installed only on the
            // open path: with no session there is no keymap to ask and
            // nothing for a command to act on, and a monitor that swallowed
            // keys to answer nothing would make the unavailable screen
            // unusable as well as empty.
            installKeyboard(session)
            // The menu bar, rendered from the same registry the palette and
            // the cheat sheet read (#657). Its accelerators come from the
            // bindings in force, and none of its items has a key equivalent:
            // dispatch is the monitor's, above.
            MenuBar.install(
                binding: { [weak self] command in self?.session?.binding(for: command) },
                run: { [weak self] id in self?.run(id) }
            )
            // The platform observes and the engine is told. Callbacks arrive
            // on a background queue and may repeat the same answer; the
            // boundary absorbs that, nudging a reconnect only on a real
            // transition back, so there is nothing to debounce here.
            reachability.start { [weak self] offline in
                Task { @MainActor in self?.session?.setOffline(offline) }
            }
        case let .failure(error):
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
    private var keys: KeyMonitor?

    /// Which surface has focus, as the resolver understands it.
    ///
    /// Reported by the views as they take focus rather than inferred from the
    /// responder chain: `UiContext` is Postio's vocabulary of surfaces and
    /// AppKit knows nothing about it, so a mapping from view classes would be
    /// this application guessing at its own state. The list is where the
    /// keyboard starts.
    var context: UiContext = .list

    /// The message the cursor is on, for the reading pane.
    ///
    /// Reported by the engine rather than read off the table, because the
    /// cursor is the boundary's: a keystroke moves it without the table
    /// having been touched at all.
    private(set) var cursorShowing: Int64?

    /// What to draw above the list — "12 selected" — or nothing.
    ///
    /// Read fresh rather than cached: it is a property of a model that a
    /// keystroke can change, and a stale count is a claim about what an
    /// action is going to hit.
    var selectionSummary: String? { session?.selectionSummary }

    /// Whether the command palette is open.
    ///
    /// A *surface*, which is why the boundary does not handle
    /// `command_palette` the way it handles `next_message`: a session cannot
    /// present a sheet. `postio-gtk`'s `run_action` makes the same call for
    /// the same reason — a frontend has to know which commands it draws
    /// something for, and only those.
    var showingPalette = false
    /// Whether the cheat sheet is open.
    var showingCheatSheet = false

    /// The chords of a half-typed sequence, for the shell to show.
    ///
    /// `nil` when nothing is pending. A sequence that is invisible while it
    /// waits is a keyboard that feels like it stopped responding.
    private(set) var pendingChord: String?

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
        session.openScope(.mailbox(mailbox: mailbox))
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
        case let .cursorMoved(row, message):
            // The table follows the model, never the other way round. `j` and
            // `k` move the cursor behind the boundary -- where the list
            // window, the selection and `aim` all are -- and this is the
            // table catching up with where it ended.
            controller.showCursor(on: row)
            cursorShowing = message
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

    /// Wire the `NSEvent` monitor to the boundary's resolver.
    ///
    /// Three lines of policy and no keymap: reduce, ask, act. The application
    /// owns which surface has focus and whether somebody is typing, because
    /// only it can see those; everything else is `postio_ui::keymap`'s.
    private func installKeyboard(_ session: PostioSession) {
        let monitor = KeyMonitor(
            resolve: { [weak self] reduced, context, typing in
                self?.session?.key(reduced, in: context, typing: typing) ?? .unhandled
            },
            run: { [weak self] id in self?.run(id) },
            pending: { [weak self] description in self?.pendingChord = description },
            context: { [weak self] in self?.context ?? .list }
        )
        monitor.start()
        keys = monitor
    }

    /// Run a command, presenting it here if it is a surface this frontend owns.
    ///
    /// The two exceptions are the two that *are* windows. Everything else —
    /// including the cursor and the selection, which are frontend state —
    /// goes to `invoke`, where the boundary decides whether it is its own or
    /// the engine's. Keeping the list to two is what stops this becoming the
    /// hand-maintained command table #657 exists to prevent.
    func run(_ id: String) {
        switch id {
        case "command_palette":
            showingCheatSheet = false
            showingPalette = true
        case "cheat_sheet":
            showingPalette = false
            showingCheatSheet = true
        case "back" where showingPalette || showingCheatSheet:
            // Escape means "get me out of here", and the innermost "here" is
            // whichever of these is open.
            showingPalette = false
            showingCheatSheet = false
        default:
            session?.invoke(id)
        }
    }

    /// Close whatever overlay is open, and put the keyboard back in the list.
    func dismissOverlays() {
        showingPalette = false
        showingCheatSheet = false
        context = .list
    }

    /// Say where the keyboard is, so a verb with nothing marked knows which
    /// row it is about.
    ///
    /// The cursor, not the selection (`PRODUCT.md` §9). This is what makes `a`
    /// archive the row being read rather than nothing at all.
    func cursorMoved(to message: Int64?) {
        session?.setCursor(message)
    }

    /// The user clicked a row.
    ///
    /// The row rather than the message, because that is what the boundary
    /// moves from: `j` after a click has to step from where the click landed.
    func cursorClicked(row: UInt32?) {
        session?.setCursorRow(row)
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
        keys?.stop()
        keys = nil
        reachability.stop()
        session?.shutdown()
        session = nil
    }
}
