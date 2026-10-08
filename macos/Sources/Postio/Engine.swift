import AppKit
import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI

/// The engine, and what to show when it will not start.
///
/// Opening a session reads the local store's key from the login Keychain
/// (ADR 0014), and an ad-hoc-signed build has a new code identity on every
/// rebuild — so macOS asks again after each one. That is a real state the
/// application has to render rather than crash through, and it is the reason
/// this holds a *result* rather than a session.
///
/// The composition root of the Focus app (specs/009-focus-macos R10): the
/// session, Focus's list, the toolbar's search field, the windows a command
/// can ask for, and the event drain. The three-pane shell's state — the
/// sidebar, the panes, the conversation and the reading pane — went with it
/// (T034); the message window brings a reader back in its own window.
// `@MainActor` because everything it holds is: the table, the views that
// read it. The engine's own work happens on its runtime, not here.
@MainActor
@Observable
final class Engine {
    enum State {
        /// The store is being opened, off this actor. See ``Engine/init()``.
        case opening
        /// A live session; Focus's list is `focusTable`.
        case open
        /// No session, and the sentence explaining why.
        case unavailable(String)
    }

    private(set) var state: State = .opening

    /// The `[ui]` table this session was opened with: the theme is drawn
    /// from it, so a change has to repaint. `nil` until a session opens,
    /// which is the window's honest state before then: it follows the
    /// system.
    private(set) var appearance: AppearanceFfi?

    /// The configured accounts, for the settings window's Accounts pane and
    /// for whether the window is the first-run wizard.
    private(set) var accounts: [AccountFfi] = []
    /// The first-run wizard, while the store has no account (canvas 09).
    /// Made once per session rather than per draw, so what was typed
    /// survives the window redrawing around it.
    private(set) var firstRun: FirstRunModel?

    /// The colour scheme `[ui].theme` asks for, or `nil` to follow the system.
    var colorScheme: ColorScheme? {
        switch appearance?.theme {
        case .light: return .light
        case .dark: return .dark
        case .system, .none: return nil
        }
    }
    private(set) var session: PostioSession?

    /// Starts the log, then opens the store **off this actor**.
    ///
    /// `Session::open` says not to call it on the main actor and means it:
    /// the store's key comes from the login Keychain, that round trip can
    /// wait on a user prompt, and `@State private var engine = Engine()` runs
    /// inside `App.init()` — before SwiftUI has a scene. Done synchronously,
    /// the application appeared in the Dock and drew no window at all
    /// (#1146). So opening is a *state*, and the window that draws it is also
    /// what the Keychain prompt appears in front of.
    init() {
        // First, so the Keychain refusing or the store refusing to migrate
        // say so somewhere rather than arriving as an empty window.
        PostioSession.startLogging()
        // Before the store, deliberately: a bar installed after it was
        // AppKit's stock one for the whole of the Keychain's wait (#1262).
        installMenuBar()
        Task.detached(priority: .userInitiated) {
            let opened = Result {
                try DemoMode.seed.map(PostioSession.openDemo) ?? PostioSession.open()
            }
            await MainActor.run { [weak self] in self?.adopt(opened) }
        }
    }

    /// Take up a session that opened, or record why one did not.
    private func adopt(_ opened: Result<PostioSession, Error>) {
        switch opened {
        case let .success(session):
            self.session = session
            appearance = session.appearance()
            accounts = session.accounts()
            if accounts.isEmpty { firstRun = FirstRunModel(session: session) }
            vouch()
            focusTable = makeFocusTable(session)
            // The toolbar was built before there were bindings to spell.
            keycapsChanged?()
            state = .open
            mailboxes = session.mailboxes
            // Nothing is fetched until an engine starts (#648); the list
            // repaints from events rather than from anything awaited here.
            // A demo never syncs: its mail is invented and its account has
            // no server (`DemoMode`).
            if DemoMode.seed == nil { _ = try? session.startSyncing() }
            // An account added while this runs gets its engine here, which is
            // what makes it sync without a relaunch (#1299).
            settingsActions.accountAdded = { [weak self] in
                guard let self, let session = self.session else { return }
                self.accounts = session.accounts()
                if !self.accounts.isEmpty { self.firstRun = nil }
                self.vouch()
                _ = try? session.startSyncing()
                self.mailboxes = session.mailboxes
            }
            notifications.start()
            // A notification click brings Postio forward, which macOS does on
            // its own. Opening the message it names is the message window's
            // (`postio://message/<id>`, later).
            notifications.open = { _, _ in }
            consumeEvents(from: session)
            openFocus(.inbox)
            // Keystrokes, resolved by the core (#656), only once there is a
            // keymap to ask: a monitor that swallowed keys to answer nothing
            // would make the unavailable screen unusable as well as empty.
            installKeyboard(session)
            // Again, now that there are bindings to draw.
            installMenuBar()
            // The platform observes and the engine is told; the boundary
            // absorbs repeats, so there is nothing to debounce here.
            reachability.start { [weak self] offline in
                Task { @MainActor in
                    self?.session?.setOffline(offline)
                    self?.vouch()
                }
            }
        case let .failure(error):
            // The boundary's sentence, not one invented here: a locked
            // keychain says how to unlock it, and a broken store says what
            // broke.
            state = .unavailable(String(describing: error))
            session = nil
        }
    }

    /// Every folder, flat: the settings window lists them, and the sync
    /// label reads when each last synced.
    private(set) var mailboxes: [MailboxFfi] = []

    /// Asked for the settings window. Watched by the main window, which is
    /// what can actually open one.
    private(set) var settingsWindow = WindowRequest(id: WindowId.settings)

    /// The messages being written, and the windows they are waiting for.
    let compose = ComposeStore()

    /// What the settings window's account actions are doing, held here
    /// because their progress arrives as events and a window that owned
    /// them would have to be open at the moment one landed.
    let settingsActions = AccountActions()

    /// Which account row the settings window's keyboard is on, and the two
    /// sheets a command can ask that window for. See `SettingsAccounts`.
    let settingsAccounts = SettingsAccounts()

    /// Putting a broken account back in service. Held here because
    /// `update_credential` is a command, and a command cannot reach a view.
    let accountRepair = AccountRepair()

    private let notifications = MailNotifications()
    private let reachability = Reachability()
    private var keys: KeyMonitor?

    /// Which window has the keyboard: the key monitor sees every key press
    /// in the application, compose windows included.
    let keyWindow = KeyWindowTracker()

    /// Which surface the resolver answers for inside the main window: the
    /// list, or the search field while it has the keyboard.
    var mainContext: UiContext { showingSearch || finding != nil ? .search : .list }

    /// Which surface the resolver should answer for. The window decides
    /// first; see `KeyboardContext`.
    var context: UiContext {
        KeyboardContext.resolving(keyWindow: keyWindow.current, mainWindow: mainContext)
    }

    // MARK: Focus's list

    /// Focus's list, once a session has opened (specs/009-focus-macos US1).
    private(set) var focusTable: FocusListTable?

    /// How many rows Focus's list draws, observed: SwiftUI decides between
    /// the list and the empty inbox on it, and the table's own count is
    /// behind the FFI where nothing can observe it.
    private(set) var focusCount: UInt32 = 0

    /// Whether the list has said how long it is yet. Until it has, an empty
    /// count is "not counted", not "empty".
    private(set) var focusListed = false

    /// Which of Focus's lists is open.
    private(set) var focusScope: FocusScopeFfi = .inbox

    /// The header strip's words, read off this actor when they may have
    /// moved.
    private(set) var focusStrip: FocusStripFfi?

    /// Bumped when `[keys]` changes, so every keycap is spelled again.
    private(set) var keymapVersion = 0

    /// The controller's cursor, selection, heading and toast, as its
    /// intents left them: what the table, the action bar and the toast
    /// line read (T049).
    let focus = FocusIntents()

    private func makeFocusTable(_ session: PostioSession) -> FocusListTable {
        let table = FocusListTable(
            model: FocusListModel(source: session, focus: focus) { [weak session] command in
                session?.binding(for: command)
            })
        // The pointer is told to the controller; the ring and the boxes move
        // when its intents come back, never here.
        table.onPoint = { [weak session] row in session?.focusPoint(row) }
        table.onPick = { [weak session] row, range in session?.focusPick(row, range: range) }
        table.onAtTop = { [weak session] atTop in session?.focusAtTop(atTop) }
        table.onAction = { [weak self, weak table] command, row in
            // An answer on a row is about that row: the controller's cursor
            // goes there first (synchronously, on the other side), so a verb
            // that aims at the cursor lands where the click did, and one
            // answered here is aimed at that row's message.
            self?.session?.focusPoint(row)
            self?.run(command, on: table?.model.row(at: row)?.id)
        }
        return table
    }

    /// The header strip's words, from the counts and the bindings in force.
    var stripWords: HeaderStripWords {
        _ = keymapVersion
        // On while the controller's `!` heading stands: the toggle is the
        // controller's (`toggle_has_action`), and its heading is how it says
        // so.
        return HeaderStripWords(strip: focusStrip, hasActionOn: focus.heading != nil) {
            [weak self] command in self?.session?.binding(for: command)
        }
    }

    /// Show one of Focus's lists. It opens on its first row (C30).
    func openFocus(_ scope: FocusScopeFfi) {
        guard let session else { return }
        focusScope = scope
        focusListed = false
        session.openFocus(scope)
        refreshCounts()
    }

    /// Whether a count read is in flight, and whether another was asked for
    /// while it was: events come in bursts during a sync, and one read per
    /// burst is the right number.
    private var countsReading = false
    private var countsOwed = false

    /// Read the strip's counts again, off this actor: they are a query.
    func refreshCounts() {
        guard let session else { return }
        guard !countsReading else {
            countsOwed = true
            return
        }
        countsReading = true
        Task {
            let strip = await Task.detached { try? session.focusStrip() }.value
            countsReading = false
            if let strip { focusStrip = strip }
            if countsOwed {
                countsOwed = false
                refreshCounts()
            }
        }
    }

    /// The message under the Focus cursor, if its row has arrived.
    var focusCursorMessage: Int64? {
        guard let table = focusTable, let row = table.cursor else { return nil }
        return table.model.row(at: row)?.id
    }

    // MARK: the toolbar

    /// Whether a sync pass is running now, from `SyncProgress`: the presence
    /// of progress is the answer to "is anything happening".
    private(set) var syncing = false

    /// How far the sync pass in flight has come.
    private(set) var syncProgress: (done: UInt32, total: UInt32)?

    /// Why an account cannot sign in, or `nil` while every one can. Not the
    /// machine's reachability, which is `isOffline`.
    private(set) var failure: FailureReasonFfi?

    /// Whether the platform has told the engine there is no connection.
    var isOffline: Bool { session?.isOffline ?? false }

    /// The toolbar's sync label.
    var syncLabel: SyncLabel {
        SyncLabel(
            offline: isOffline,
            failing: failure != nil,
            syncing: syncing ? syncProgress : nil,
            lastSynced: mailboxes.compactMap(\.lastSyncedAt).max()
        )
    }

    /// The toolbar's search field, as the engine reaches it: its text, the
    /// keyboard, and its keycap's spelling. The field is AppKit's, so the
    /// toolbar hands these over when it is installed.
    var fieldText: ((String) -> Void)?
    var focusField: (() -> Void)?
    var keycapsChanged: (() -> Void)?

    /// Whether the search field has the keyboard. Moves the key context with
    /// it: a key typed there must not resolve as the list.
    var showingSearch = false

    /// The question the search field is asking -- `>`, `#`, `@` or `+` and
    /// what follows it -- or `nil` while it is a search (`FinderBox`).
    private(set) var finding: FinderBox.Asking?
    /// Which row of the answer the keyboard is on.
    private(set) var finderBox = FinderBox()
    /// The rows for what the field is asking, and what to say when there
    /// are none. Read once per change of the text, not per draw.
    private(set) var finderAnswer = (rows: [FinderRow](), empty: "")

    /// Put `text` in the search field.
    private func askField(_ text: String) {
        fieldText?(text)
    }

    /// The field's report: what it is asking, or `nil` when it is a search.
    func findingChanged(_ asking: FinderBox.Asking?) {
        guard asking != finding else { return }
        finding = asking
        finderBox.queryChanged()
        finderAnswer = answer(for: asking)
    }

    private func answer(for asking: FinderBox.Asking?) -> (rows: [FinderRow], empty: String) {
        guard let asking, let session else { return ([], "") }
        switch asking.mode {
        case .command:
            let rows = session.paletteEntries(asking.text, in: .list).map {
                FinderRow(id: $0.id, title: $0.title, detail: nil, positions: $0.positions, binding: $0.binding)
            }
            return (rows, "No command matches “\(asking.text)”")
        case .folder:
            return rows(of: session.finderFolders(asking.text))
        case .contact:
            return rows(of: session.finderContacts(asking.text))
        case .label:
            return rows(of: session.finderLabels(asking.text))
        }
    }

    private func rows(of answer: FinderAnswerFfi) -> (rows: [FinderRow], empty: String) {
        let rows = answer.hits.map {
            FinderRow(
                id: $0.query ?? String($0.id), title: $0.title, detail: $0.detail,
                positions: $0.positions, binding: nil)
        }
        return (rows, answer.empty)
    }

    /// ↑ or ↓ in the field.
    func moveFinder(by delta: Int) {
        finderBox.move(by: delta, among: finderAnswer.rows.count)
    }

    /// Return in the field: the highlighted row.
    func pickHighlighted() {
        pick(finderBox.highlighted)
    }

    /// A row chosen, by Return or by a click.
    func pick(_ index: Int) {
        guard let asking = finding, index < finderAnswer.rows.count, let session else { return }
        let row = finderAnswer.rows[index]
        leaveFinder()
        dismissOverlays()
        switch asking.mode {
        case .command:
            run(row.id)
        case .label:
            if let id = Int64(row.id) { session.applyLabel(id) }
        case .folder, .contact:
            // Focus's places and a correspondent's mail open in the command
            // bar's lists, which come with it (FR-013). A pick that appeared
            // to work and showed nothing would be worse than a beep.
            NSSound.beep()
        }
    }

    private func leaveFinder() {
        guard finding != nil else { return }
        finding = nil
        finderAnswer = ([], "")
        askField("")
    }

    // MARK: what Postio says back

    /// Whether the key map is open.
    var showingCheatSheet = false

    /// The chords of a half-typed sequence, shown while it waits.
    private(set) var pendingChord: String?

    /// What Postio last said back about something you asked it to do.
    private(set) var notice: Notice?

    /// Bumped whenever `notice` is set, so a second identical sentence is a
    /// second notice (*Archived* twice is two things happening).
    private(set) var noticeToken = 0

    /// Take the notice down.
    func dismissNotice() {
        notice = nil
    }

    /// Say what went wrong, if anything did.
    private func complain(_ said: String?) {
        guard let said else { return }
        notice = Notice(kind: .refused, message: said, undoable: false)
        noticeToken += 1
    }

    // MARK: accounts

    /// Re-read the accounts after one of them changed. The rows are the
    /// boundary's answer, not a local copy to patch.
    func refreshAccounts() {
        guard let session else { return }
        accounts = session.accounts()
        // Removing the last account is a fresh install again.
        if accounts.isEmpty, firstRun == nil {
            firstRun = FirstRunModel(session: session)
        } else if !accounts.isEmpty {
            firstRun = nil
        }
        vouch()
    }

    /// Tell the boundary which accounts the unified view can vouch for.
    /// See `VouchedFor`.
    private func vouch() {
        session?.setReachableAccounts(VouchedFor.accounts(accounts, offline: session?.isOffline ?? true))
    }

    /// Open `config.toml` in whatever edits it. The path is
    /// `postio-config`'s; created empty if it is not there yet, because a
    /// first run has none and `NSWorkspace` cannot open what does not exist.
    private func openConfigFile() {
        guard let path = try? settingsPath() else {
            complain("Postio could not work out where its configuration file lives.")
            return
        }
        if !FileManager.default.fileExists(atPath: path) {
            FileManager.default.createFile(atPath: path, contents: Data())
        }
        // POSTIO-CONSENT: `⌘E` — *Edit configuration* — and nothing else.
        // What is handed out is Postio's own settings file, to whichever
        // application the user has told macOS edits TOML; no message, no
        // address, and no URL from anybody's mail is involved.
        NSWorkspace.shared.open(URL(fileURLWithPath: path))
    }

    // MARK: events

    /// Drain the engine's events for as long as the session is open. The
    /// task ends when `nextEvent` answers `nil`, which `shutdown` makes it do.
    private func consumeEvents(from session: PostioSession) {
        Task { @MainActor [weak self] in
            while let event = await session.nextEvent() {
                self?.handle(event)
            }
        }
    }

    /// React to one engine event. The `default:` arm is ADR 0019 Q7's: the
    /// event union is append-only, so an older build ignores what it does
    /// not know rather than failing on it.
    private func handle(_ event: UiEvent) {
        guard case .open = state else { return }
        // Before the switch: what the application says back is not one
        // arm's business.
        if let arriving = Notice(event) {
            notice = Notice.winner(showing: notice, arriving: arriving)
            noticeToken += 1
        }
        // The controller's intents: the cursor, the selection, `!`'s heading,
        // the toast (T049). The table draws what changed.
        if let change = focus.apply(event) {
            focusTable?.apply(change)
            return
        }
        switch event {
        case let .focusListChanged(total):
            focusCount = total
            focusListed = true
            focusTable?.listChanged(total: total)
            refreshCounts()
        case let .focusPageReady(page):
            focusTable?.pageArrived(page)
        case .keymapChanged:
            keymapVersion += 1
            focusTable?.keymapChanged()
            keycapsChanged?()
            installMenuBar()
        case .surfacedChanged:
            // The list re-reads what it surfaces and says so itself; the
            // strip's counts may have moved with it.
            refreshCounts()
        case let .newMail(account, mailbox, messages):
            mailMoved()
            arrived(MailArrival(account: account, mailbox: mailbox, messages: messages))
        case .messagesChanged, .messagesRemoved, .messageListChanged, .mailboxesChanged:
            // Read state, mail leaving, the folder tree: the settings
            // window's folders and the strip's counts move with them.
            mailMoved()
        case let .reindexProgress(_, done, total):
            // The settings window asked for this, and it is the only thing
            // that draws it.
            settingsActions.reindexProgressed(done: done, total: total)
        case let .syncProgress(_, done, total):
            syncing = done < total
            syncProgress = syncing ? (done, total) : nil
        case let .connectionChanged(_, state):
            // A connection that has gone means nothing is in flight.
            if isOffline { syncing = false }
            // And the reason is kept: an expired password must not read as
            // "synced" for as long as you leave it.
            switch state {
            case let .failing(reason):
                failure = reason
                syncing = false
            case .online, .connecting, .offline:
                failure = nil
            }
        default:
            break
        }
    }

    /// Mail arrived, left, or changed state.
    private func mailMoved() {
        mailboxes = session?.mailboxes ?? []
        refreshCounts()
    }

    /// Decide what to do about new mail, and do it.
    private func arrived(_ arrival: MailArrival) {
        let decision = MailNotifier.decide(
            arrival,
            // Focus's inbox is not one folder, so no arrival is "already on
            // screen" by that rule; the application being in front is.
            showing: nil,
            isActive: NSApplication.shared.isActive,
            mailboxName: mailboxes.first { $0.id == arrival.mailbox }?.name
        )
        guard case let .deliver(notification) = decision else { return }
        notifications.post(notification)
    }

    // MARK: keys and the menu bar

    /// Build the menu bar from the registry and hang it off `NSApp`
    /// (#657). Accelerators come from the bindings in force where there is
    /// a session to ask and from the built-in defaults before there is one;
    /// dispatch is the key monitor's.
    private func installMenuBar() {
        MenuBar.install(
            bindings: { [weak self] command in self?.session?.bindings(for: command) ?? [] },
            available: { [weak self] id in
                guard let self else { return false }
                guard let session else {
                    // Before a session, the verbs this frontend handles itself
                    // still work: Settings edits a file.
                    return Intercepted.all.contains(id)
                }
                return session.isAvailable(id, in: self.context)
            },
            run: { [weak self] id in self?.run(id) }
        )
    }

    /// Wire the `NSEvent` monitor to the boundary's resolver: reduce, ask,
    /// act. The keymap is `postio_ui::keymap`'s.
    private func installKeyboard(_ session: PostioSession) {
        let monitor = KeyMonitor(
            resolve: { [weak self] reduced, context, typing in
                self?.session?.key(reduced, in: context, typing: typing) ?? .unhandled
            },
            run: { [weak self] id in self?.run(id) ?? false },
            pending: { [weak self] description in self?.pendingChord = description },
            context: { [weak self] in self?.context ?? .list }
        )
        monitor.start()
        keys = monitor
    }

    /// Run a command, presenting it here if it is a surface this frontend
    /// owns; everything else goes to `invoke`, where the boundary decides.
    @discardableResult
    func run(_ id: String, on target: Int64? = nil) -> Bool {
        switch id {
        case Intercepted.palette:
            // The search field, asked for a command: the commands appear
            // under it as the name is typed (`FinderBox`).
            showingCheatSheet = false
            askField(FinderBox.commands)
            showingSearch = true
            focusField?()
        case Intercepted.cheatSheet:
            leaveFinder()
            showingCheatSheet = true
        case Intercepted.search:
            showingSearch = true
            focusField?()
        case Intercepted.back where finding != nil:
            // Out of command mode and out of the field: `>` was a question,
            // and Escape is "never mind".
            leaveFinder()
            dismissOverlays()
        case Intercepted.back where showingCheatSheet:
            showingCheatSheet = false
        case Intercepted.back where showingSearch:
            dismissOverlays()
        case Intercepted.settings:
            // A request the main window turns into `openWindow(id:)`,
            // because only a view can open a window (#1261).
            settingsWindow.raise()
        case Intercepted.compose:
            write(session?.newDraft())
        case Intercepted.reply:
            write(replyDraft(all: false, to: target))
        case Intercepted.replyAll:
            write(replyDraft(all: true, to: target))
        case Intercepted.forward:
            guard let session, let message = target ?? focusCursorMessage else { return false }
            write(session.forwardDraft(message))
        // -- the settings window's accounts pane ------------------------
        //
        // All aim at the row that window's keyboard is on, and a missing
        // cursor is a real answer: falling back to "the first account"
        // would remove somebody's mail on a keystroke aimed at nothing.
        case Intercepted.addAccount:
            settingsWindow.raise()
            settingsAccounts.ask(.add)
        case Intercepted.editConfig:
            openConfigFile()
        case Intercepted.updateCredential:
            guard let account = settingsAccounts.focused(in: accounts) else { return false }
            settingsAccounts.ask(.updateCredential(account.id))
        case Intercepted.toggleAccountEnabled:
            guard let account = settingsAccounts.focused(in: accounts) else { return false }
            complain(session?.setAccountEnabled(account.id, !account.enabled))
            refreshAccounts()
        case Intercepted.setDefaultAccount:
            guard let account = settingsAccounts.focused(in: accounts) else { return false }
            complain(session?.setDefaultAccount(account.id))
            refreshAccounts()
        case Intercepted.removeAccount:
            // Asked about, never done: it takes the account's mail with it.
            guard let account = settingsAccounts.focused(in: accounts) else { return false }
            settingsActions.askToRemove(account)
        case Intercepted.rebuildAccountIndex:
            guard let account = settingsAccounts.focused(in: accounts) else { return false }
            let session = session
            Task { await settingsActions.reindex(account, through: session) }
        case Intercepted.quit:
            // Through AppKit, so the delegate's termination path runs as for
            // `⌘Q`.
            NSApplication.shared.terminate(nil)
        default:
            // A compose window in front gets first refusal on the composer's
            // own verbs — and only the one with the keyboard.
            if keyWindow.current == .compose,
               let draft = keyWindow.currentDraft,
               let composer = compose.model(draft),
               ComposeCommands.run(id, on: composer, through: session)
            {
                return true
            }
            session?.invoke(id)
        }
        return true
    }

    /// Open a compose window for `draft`, or say why there is none: on a
    /// fresh install there is no account to write from, and a `⌘N` that
    /// appeared to do nothing is a bug this port has produced three times.
    private func write(_ draft: DraftFfi?) {
        guard let draft else {
            NSSound.beep()
            return
        }
        compose.open(draft)
    }

    /// Open a composer on a `mailto:` link. `false` when there is nothing to
    /// open one from, which the caller says out loud.
    func write(mailto: Mailto) -> Bool {
        guard let session, let draft = session.mailtoDraft(mailto) else { return false }
        compose.open(draft)
        return true
    }

    /// A reply to `target`, or to the message the cursor is on — the cursor,
    /// not the selection: replying to twelve marked messages is not a thing.
    private func replyDraft(all: Bool, to target: Int64? = nil) -> DraftFfi? {
        guard let session, let message = target ?? focusCursorMessage else { return nil }
        return session.replyDraft(to: message, all: all)
    }

    /// Close whatever overlay is open, and give the keyboard back to the
    /// list.
    func dismissOverlays() {
        showingCheatSheet = false
        showingSearch = false
        if let table = focusTable?.tableView { table.window?.makeFirstResponder(table) }
    }

    /// Stop the engines and drop the store, in that order. Called from the
    /// application's termination handler: the store is SQLCipher, and
    /// dropping an engine at process exit is when libcrypto goes away under
    /// a thread still encrypting a page.
    func shutdown() {
        keys?.stop()
        keys = nil
        reachability.stop()
        session?.shutdown()
        session = nil
    }
}
