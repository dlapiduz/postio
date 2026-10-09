import AppKit
import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI
import os

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
        /// No session: the store was refused, and the page that says why
        /// and offers the way forward (T100).
        case refused(StoreRefusalModel)
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
        openStore()
    }

    /// Open the store off this actor, then take up what opened -- at
    /// launch, and again from the refusal page's button.
    private func openStore(after started: StartedOverFfi? = nil) {
        state = .opening
        Task.detached(priority: .userInitiated) {
            let opened = Result {
                try DemoMode.seed.map(PostioSession.openDemo) ?? PostioSession.open()
            }
            await MainActor.run { [weak self] in
                self?.adopt(opened)
                // Where the old store went, once the fresh one is open: it
                // was set aside, not deleted, and this is how anyone finds
                // it again (GTK says the same).
                if let started, let self, case .open = self.state {
                    self.notice = Notice(
                        kind: .completed, message: startedOverWords(setAside: started.setAside),
                        undoable: false)
                    self.noticeToken += 1
                }
            }
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
            messageWindow = MessageWindowModel(source: session) { [weak session] more, finding in
                session?.focusReaderState(moreOpen: more, finding: finding)
            }
            makeBar(session)
            makePicker(session)
            filtered = FilteredModel(engine: session)
            makeResults(session)
            digest = DigestModel(engine: session, source: session)
            ruleSheet = RuleSheetModel(engine: session)
            capture = CaptureModel(engine: session)
            composer = ComposerWindow(engine: session)
            // The toolbar was built before there were bindings to spell.
            keycapsChanged?()
            state = .open
            mailboxes = session.mailboxes
            // Nothing is fetched until an engine starts (#648); the list
            // repaints from events rather than from anything awaited here.
            // A demo never syncs: its mail is invented and its account has
            // no server (`DemoMode`).
            if DemoMode.seed == nil { _ = try? session.startSyncing() }
            // What sync would have said, for screens 16 to 19 (T101).
            if let said = DemoMode.state { _ = session.demoState(said) }
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
            // The stack's top may have moved while another window had the
            // keyboard, or an entry's window closed while nobody looked.
            NotificationCenter.default.addObserver(
                forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main
            ) { [weak self] note in
                guard let window = note.object as? NSWindow else { return }
                MainActor.assumeIsolated {
                    guard KeyWindowTracker.isMain(window) else { return }
                    self?.refreshUndo()
                }
            }
            refreshUndo()
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
            // broke. A store from another build cannot be got past by trying
            // again, so its page offers a fresh store instead (T100). The
            // case only, in the log: the sentence may name a path.
            Self.log.error("the store did not open")
            let words = storeRefusalWords()
            session = nil
            state = .refused(
                StoreRefusalModel(
                    refusal: StoreRefusal(error, words: words), words: words,
                    // `start_over` blocks on the Keychain and the disk; the
                    // model calls it off this actor. The usual path, as
                    // `PostioSession.open` opens.
                    startOver: { try PostioFFI.startOver(storePath: nil) },
                    reopen: { [weak self] started in self?.openStore(after: started) }))
        }
    }

    /// Every folder, flat: the settings window lists them, and the sync
    /// label reads when each last synced.
    private(set) var mailboxes: [MailboxFfi] = []

    /// Asked for the settings window. Watched by the main window, which is
    /// what can actually open one.
    private(set) var settingsWindow = WindowRequest(id: WindowId.settings)

    // MARK: the composer (T079)

    /// The composer, as the controller's intents leave it: one at a time,
    /// in the secondary window (M4).
    private(set) var composer: ComposerWindow?

    /// The address book, lent to recipient completion after one prompt
    /// (T078). Read for each answer and kept nowhere.
    @ObservationIgnored
    private lazy var contacts = ContactsSource(book: SystemContactBook())

    /// How wide the composer is over a 1440 main window (screen 05): wider
    /// than the message window, since it is written in, not read.
    static let composerWidth: CGFloat = 980

    /// What the controller said about the composer.
    private func apply(_ change: ComposerWindow.Change) {
        switch change {
        case let .open(kind, message):
            openComposer(kind, answering: message)
        case let .save(composition):
            guard let session else { return }
            composer?.save(composition) { session.saveDraft($0) }
        case .close:
            secondary.close(.composer)
        }
    }

    /// Make the draft the controller asked for and show it. With no draft
    /// to show -- no account to write from, a message gone -- the stack is
    /// put right and nothing opens.
    private func openComposer(_ kind: ComposerKindFfi, answering message: Int64?) {
        guard let session, let composer else { return }
        let draft: DraftFfi?
        switch kind {
        case .new: draft = session.newDraft()
        case .reply: draft = message.flatMap { session.replyDraft(to: $0, all: false) }
        case .replyAll: draft = message.flatMap { session.replyDraft(to: $0, all: true) }
        case .forward: draft = message.flatMap { session.forwardDraft($0) }
        case .draft: draft = message.flatMap { session.draftForMessage($0) }
        }
        guard let draft else {
            NSSound.beep()
            composer.couldNotOpen()
            return
        }
        composer.show(draft)
        showComposerWindow()
    }

    /// The composer's window over the main window (M4): in the open one,
    /// refilled, or a new one as wide as screen 05's.
    private func showComposerWindow() {
        guard let composer, let model = composer.model, let session, let main = mainWindow else { return }
        let close: () -> Void = { [weak self] in self?.secondary.close(.composer) }
        let content = ComposeView(
            // A demo never reads the address book: its mail is invented, and
            // a photograph is no time for the system's prompt.
            session: session, model: model, accounts: accounts,
            contacts: DemoMode.seed == nil ? contacts : nil,
            edited: { [weak composer] in composer?.edited() },
            close: close
        )
        .preferredColorScheme(colorScheme)
        let hosting = NSHostingView(rootView: content)
        hosting.sizingOptions = []
        // Send and Send later close the window once the draft is on its
        // way; the toolkit's close says so to the controller.
        let chrome = ComposeWindowChrome(
            model: model,
            sendCap: KeyCapSpelling.cap(session.binding(for: "send")),
            send: { [weak session, weak model] in
                guard let session, let model, model.send(through: session) else { return }
                close()
            },
            sendAt: { [weak session, weak model] when in
                guard let session, let model else { return }
                model.send(at: when, through: session)
                if model.sent { close() }
            })
        composeChrome = chrome
        let width = min(Self.composerWidth, max(SecondaryWindowController.minimumHeight, main.frame.width - 80))
        secondary.show(
            .composer, content: hosting, width: width, title: model.title, over: main,
            configure: { window in
                KeyWindowTracker.tag(window, as: .compose, draft: model.id)
                chrome.install(on: window)
            })
        // The same kind again keeps its window and its toolbar: the title
        // area reads the new draft.
        if let window = secondary.window, secondary.kind == .composer {
            chrome.install(on: window)
        }
    }

    /// The composer's title area, while it is open.
    @ObservationIgnored private var composeChrome: ComposeWindowChrome?

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

    /// The same, for the sign-in banner's "Update password…" (screen 19):
    /// a sheet on the main window rather than the settings window's alert.
    /// Its own, because both windows present whenever theirs is asking.
    let bannerRepair = AccountRepair()

    /// The account the banner's password sheet is for, while it is up.
    var bannerRepairAccount: AccountFfi? {
        guard let id = bannerRepair.asking else { return nil }
        return accounts.first { $0.id == id }
    }

    /// Edit › Undo: the engine's stack in the main window (T051).
    @ObservationIgnored
    private lazy var undoRouter = UndoRouter(
        manager: PostioUndoManager { [weak self] in
            self?.session?.invoke(Notice.undoCommand)
        })

    /// Read what Undo would take back, off this actor, for the Edit menu.
    private func refreshUndo() {
        guard let session else { return }
        let manager = undoRouter.manager
        Task { await manager.refresh { session.undoDescription() } }
    }

    private let notifications = MailNotifications()
    private let reachability = Reachability()
    private var keys: KeyMonitor?

    /// Which window has the keyboard: the key monitor sees every key press
    /// in the application, compose windows included.
    let keyWindow = KeyWindowTracker()

    /// Which surface the resolver answers for inside the main window: the
    /// list, or the command bar while it is up. The controller answers with
    /// its own context while a surface it knows of is over the list; this
    /// is what a menu greys against.
    var mainContext: UiContext {
        if commandBar?.isOpen == true { return .search }
        if results?.isOpen == true { return .results }
        if capture?.isOpen == true { return .capture }
        if digest?.isOpen == true { return .digest }
        if filtered?.isOpen == true { return .filtered }
        return .list
    }

    /// Which surface the resolver should answer for. The window decides
    /// first; see `KeyboardContext`.
    var context: UiContext {
        KeyboardContext.resolving(keyWindow: keyWindow.current, mainWindow: mainContext)
    }

    // MARK: Focus's list

    /// Focus's list, once a session has opened (specs/009-focus-macos US1).
    private(set) var focusTable: FocusListTable?

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

    /// The action bar's words, while anything is selected.
    var actionBarWords: ActionBarWords? {
        _ = keymapVersion
        // `vault: false`: the boundary does not say yet whether capture has
        // a vault to write to (C9), and a Task button that can only fail is
        // worse than none.
        return ActionBarWords(
            summary: focus.summary, hasSelection: focus.hasSelection, vault: false
        ) { [weak self] command in self?.session?.binding(for: command) }
    }

    /// The keycap the toast line's Undo shows.
    var undoCap: String? {
        _ = keymapVersion
        return KeyCapSpelling.cap(session?.binding(for: Notice.undoCommand))
    }

    /// The header strip's words, from the counts and the bindings in force.
    var stripWords: HeaderStripWords {
        _ = keymapVersion
        // On while the controller's `!` heading stands: the toggle is the
        // controller's (`toggle_has_action`), and its heading is how it says
        // so.
        return HeaderStripWords(
            strip: focusStrip, place: places?.placeName ?? "Inbox", hasActionOn: focus.heading != nil
        ) {
            [weak self] command in self?.session?.binding(for: command)
        }
    }

    /// Show one of Focus's lists. It opens on its first row (C30).
    func openFocus(_ scope: FocusScopeFfi) {
        guard let session else { return }
        focusScope = scope
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

    // MARK: the message window

    /// The message window's state (specs/009-focus-macos US3), once a
    /// session has opened.
    private(set) var messageWindow: MessageWindowModel?

    /// The one secondary window over the list (M4).
    @ObservationIgnored
    private lazy var secondary = SecondaryWindowController { [weak self] kind in
        self?.secondaryClosed(kind)
    }

    /// The message window's title area, while it is open.
    @ObservationIgnored
    private var messageChrome: MessageWindowChrome?

    /// Whether the engine has been told the message window is open, so a
    /// close is reported only for an open that was.
    @ObservationIgnored
    private var messageReported = false

    private static let log = Logger(subsystem: "dev.postio.Postio", category: "focus")

    /// The main window: the one the list is in.
    private var mainWindow: NSWindow? {
        focusTable?.tableView.window ?? NSApp.windows.first { KeyWindowTracker.isMain($0) }
    }

    /// Run `command` from the settings window, with the main window in
    /// front: Filtering's Open Filtered draws Filtered in the list's place,
    /// which the settings window would otherwise hide.
    func runFromSettings(_ command: String) {
        mainWindow?.makeKeyAndOrderFront(nil)
        run(command)
    }

    /// Do what the controller said about the windows over the list (T070).
    private func apply(_ surface: FocusIntents.Surface) {
        switch surface {
        case let .openMessage(message, index, total):
            openMessage(message, index: index, total: total)
        case let .close(kind):
            if kind == .message, secondary.kind != .message {
                // Asked before its window was up: nothing on screen to close.
                messageWindow?.closed()
                if messageReported { session?.focusSurfaceClosed(.message) }
                messageReported = false
            } else {
                secondary.close(kind)
            }
        case let .reader(verb):
            messageWindow?.apply(verb)
        case .keyboardHome where results?.isOpen == true:
            // In the results, home is the results table, or the Files
            // tab's grid.
            mainWindow?.makeKeyAndOrderFront(nil)
            if results?.isFiles == true { filesGrid?.takeKeyboard() } else { resultsTable?.takeKeyboard() }
        case .keyboardHome:
            guard let table = focusTable?.tableView, let window = table.window else { return }
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(table)
        case .openDigest:
            // `DigestModel` hears `FocusOpenDigest` first and opens the
            // digest's window (T114); this is reached only before a session
            // has made one, when there is no window to open.
            break
        }
    }

    /// Show `message` in the message window: in the open one, or in a new
    /// one once its document says how wide (M1).
    private func openMessage(_ message: Int64, index: UInt32, total: UInt32) {
        guard let model = messageWindow, let session, let main = mainWindow else { return }
        let fresh = model.open(
            message: message, index: index, total: total, mainWidth: Int32(main.frame.width))
        Task { @MainActor [weak self, weak model] in
            await model?.settled()
            guard let self, let model, model.place?.message == message else { return }
            if fresh {
                self.showMessageWindow(model, session: session, over: main)
            } else {
                self.secondary.window?.title = model.view?.subject ?? ""
            }
        }
    }

    private func showMessageWindow(_ model: MessageWindowModel, session: PostioSession, over main: NSWindow) {
        guard let document = model.document else { return }
        let binding: (String) -> String? = { [weak session] in session?.binding(for: $0) }
        let run: (String) -> Void = { [weak self] in self?.run($0) }
        let chrome = MessageWindowChrome(model: model, binding: binding, run: run)
        messageChrome = chrome
        let content = MessageWindowView(
            model: model, binding: binding, run: run,
            alwaysForSender: { [weak session, weak model] in
                model?.keepForSender { session?.alwaysTreatment(sender: $0, treatment: $1) }
            },
            verbFrames: { [weak self] in self?.verbFrames = $0 }
        ) { [weak session, weak model] document, card in
            MessageBodyView(
                message: model?.shown ?? 0,
                document: document,
                sentence: card?.sentence,
                find: model?.find.request,
                onFound: { model?.found($0) },
                resolveCid: { session?.resolveCid(message: $0, contentId: $1) },
                resolveFont: { session?.readerFont($0) })
        }
        .preferredColorScheme(colorScheme)
        let hosting = NSHostingView(rootView: content)
        // The window's size is the geometry's (M1), never the content's: a
        // hosting view left to size its window grows it to the column.
        hosting.sizingOptions = []
        messageContent = hosting
        secondary.show(
            .message, content: hosting,
            width: CGFloat(document.windowWidth), title: model.view?.subject ?? "", over: main,
            configure: { chrome.install(on: $0) })
        messageReported = true
        session.focusSurfaceOpened(.message)
    }

    /// A secondary window closed, however it did: tell the engine, which
    /// sends the keyboard home.
    private func secondaryClosed(_ kind: SurfaceKindFfi) {
        if kind == .capture {
            if capture?.closedByToolkit() == true { session?.focusSurfaceClosed(.capture) }
            return
        }
        if kind == .composer {
            composeChrome = nil
            // The close button or ⌘W, or Send and Discard closing it: the
            // controller ends the composition (and asks for its save).
            if composer?.closedByToolkit() == true { session?.focusSurfaceClosed(.composer) }
            return
        }
        if kind == .digest {
            digestChrome = nil
            // Only a close the toolkit made -- the close button, ⌘W -- is
            // said: the controller's own `FocusCloseSurface` already left.
            if digest?.closedByToolkit() == true { session?.focusSurfaceClosed(.digest) }
            return
        }
        guard kind == .message else { return }
        messageChrome = nil
        messageWindow?.closed()
        if messageReported { session?.focusSurfaceClosed(.message) }
        messageReported = false
    }

    // MARK: the app's own state (T099)

    /// The banner, the toolbar's sync label and the empty page, as the
    /// controller last said them (`FocusBanner`, `FocusSyncLabel`,
    /// `FocusEmpty`). The words are `postio_ui::focus_state`'s; nothing
    /// here composes them.
    let states = FocusStates()

    /// Whether the platform has told the engine there is no connection.
    var isOffline: Bool { session?.isOffline ?? false }

    /// The keycap in the toolbar's search field, spelled again when the
    /// bindings change. The toolbar hands this over when it is installed.
    var keycapsChanged: (() -> Void)?

    /// The bar opened (`true`) or closed: the toolbar grows its search field
    /// to 860 wide while it is up, and back after (spec 010 T054).
    var barShown: ((Bool) -> Void)?

    // MARK: the command bar and the folders popover (T085, T086)

    /// The command bar's state, as the controller's intents leave it.
    private(set) var commandBar: CommandBarModel?
    /// The folders popover's state, and the place the list shows.
    private(set) var places: PlacesModel?
    /// The panel the bar is drawn in, under the toolbar's field.
    @ObservationIgnored private var barPanel: CommandBarPanel?
    /// The popover the places are listed in, under Inbox ▾.
    @ObservationIgnored private var placesPopover: PlacesPopover?
    /// The toolbar's search field: the bar's, which keeps the keyboard.
    @ObservationIgnored weak var searchField: BarSearchField?
    /// The strip's Inbox ▾, which the popover hangs from.
    @ObservationIgnored weak var placesAnchor: NSView?

    // MARK: Filtered (T113)

    /// Filtered, as the controller's intents leave it: drawn in the list's
    /// place while it is up.
    private(set) var filtered: FilteredModel?

    // MARK: the digest's window (T114)

    /// The digest's window, as the controller's intents leave it.
    private(set) var digest: DigestModel?
    /// Its title area, while it is open.
    @ObservationIgnored private var digestChrome: DigestWindowChrome?

    /// What the controller said about the digest's window.
    private func apply(_ change: DigestModel.Change) {
        switch change {
        case .open:
            showDigest()
        case .redraw:
            // The views read the model; the window's title is for the
            // Window menu and VoiceOver.
            if let title = digest?.view?.title, secondary.kind == .digest {
                secondary.window?.title = title
            }
        case .close:
            secondary.close(.digest)
        }
    }

    /// Open the digest's window over the main window, the message window's
    /// size with a 560 column (M1), replacing whatever secondary window is
    /// up (M4).
    private func showDigest() {
        guard let model = digest, let session, let main = mainWindow else { return }
        let width = Int32(main.frame.width)
        model.mainWidth = width
        let geometry = focusDigestGeometry(mainWidth: width)
        let binding: (String) -> String? = { [weak session] in session?.binding(for: $0) }
        let run: (String) -> Void = { [weak self] in self?.run($0) }
        let chrome = DigestWindowChrome(model: model)
        digestChrome = chrome
        let content = DigestWindowView(
            model: model, column: CGFloat(geometry.columnWidth), binding: binding, run: run
        ) { [weak session, weak model] document, highlight in
            MessageBodyView(
                message: model?.emailView?.message ?? 0,
                document: document,
                sentence: highlight,
                find: nil,
                onFound: { _ in },
                resolveCid: { session?.resolveCid(message: $0, contentId: $1) },
                resolveFont: { session?.readerFont($0) })
        }
        .preferredColorScheme(colorScheme)
        let hosting = NSHostingView(rootView: content)
        // The window's size is the geometry's (M1), never the content's.
        hosting.sizingOptions = []
        secondary.show(
            .digest, content: hosting, width: CGFloat(geometry.windowWidth),
            title: model.view?.title ?? "", over: main,
            configure: { chrome.install(on: $0) })
    }

    // MARK: the digest-this-sender sheet (T115)

    /// The sheet, as the controller's intents leave it.
    private(set) var ruleSheet: RuleSheetModel?

    /// The sheet's window, on the window with the keyboard.
    @ObservationIgnored
    private lazy var ruleSheetWindow = FocusSheet { [weak self] in
        guard let self, self.ruleSheet?.closedByToolkit() == true else { return }
        self.session?.focusSurfaceClosed(.dialog)
    }

    /// What the controller said about the sheet.
    private func apply(_ change: RuleSheetModel.Change) {
        switch change {
        case .open:
            guard let model = ruleSheet else { return }
            // On the digest's window when `d` was pressed there (it edits
            // that digest's rule), else on the main window.
            let digestWindow = secondary.kind == .digest ? secondary.window : nil
            guard let window = digestWindow?.isKeyWindow == true ? digestWindow : mainWindow else { return }
            let back = KeyCapSpelling.cap(session?.binding(for: Intercepted.back))
            ruleSheetWindow.show(
                DigestRuleSheet(model: model, backCap: back).preferredColorScheme(colorScheme),
                on: window)
        case .redraw:
            break
        case .close:
            ruleSheetWindow.close()
        }
    }

    // MARK: capture (T116)

    /// Capture, as the controller's intents leave it.
    private(set) var capture: CaptureModel?

    /// How tall capture's window is: screen 25's, a form rather than a page.
    private static let captureHeight: CGFloat = 620

    /// What the controller said about capture.
    private func apply(_ change: CaptureModel.Change) {
        switch change {
        case .open:
            guard let model = capture, let main = mainWindow else { return }
            let hosting = NSHostingView(
                rootView: CaptureView(model: model).preferredColorScheme(colorScheme))
            hosting.sizingOptions = []
            // A secondary window (M4): it replaces the message or digest
            // window it was opened from.
            secondary.show(
                .capture, content: hosting, width: CaptureView.width, height: Self.captureHeight,
                title: model.view?.field ?? "", over: main)
        case .redraw:
            break
        case .close:
            secondary.close(.capture)
        }
    }

    // MARK: questions (FocusConfirm)

    /// The question being asked, while its alert is up: every key is the
    /// alert's until it is answered.
    @ObservationIgnored private var asking: ConfirmQuestion?

    /// Ask what the controller asked, as a sheet on the window with the
    /// keyboard: yes is `focus_confirmed`, Cancel is nothing.
    private func ask(_ question: ConfirmQuestion) {
        guard let session else { return }
        let alert = NSAlert()
        alert.messageText = question.heading
        alert.informativeText = question.body
        let yes = alert.addButton(withTitle: question.confirm)
        yes.hasDestructiveAction = question.destructive
        alert.addButton(withTitle: "Cancel")
        asking = question
        let answer: (NSApplication.ModalResponse) -> Void = { [weak self, weak session] response in
            self?.asking = nil
            guard let session else { return }
            question.answer(response == .alertFirstButtonReturn, to: session)
        }
        if let window = NSApp.keyWindow ?? mainWindow {
            alert.beginSheetModal(for: window, completionHandler: answer)
        } else {
            answer(alert.runModal())
        }
    }

    // MARK: the pickers at the row (T092)

    /// The picker up, as the controller's intents leave it.
    private(set) var picker: PickerModel?
    /// The popover it is drawn in, under its row or its button.
    @ObservationIgnored private var pickerPopover: PickerPopover?
    /// The message window's content, and where its action row's buttons
    /// are in it: what a picker from the open message hangs from.
    @ObservationIgnored private weak var messageContent: NSView?
    @ObservationIgnored private var verbFrames: [String: CGRect] = [:]

    private func makePicker(_ session: PostioSession) {
        let picker = PickerModel(engine: session)
        self.picker = picker
        // A click outside, or the popover giving up on its own: the
        // controller is told, and sends the keyboard home.
        pickerPopover = PickerPopover(model: picker) { [weak session] in
            session?.focusSurfaceClosed(.picker)
        }
    }

    /// What the controller said about the picker.
    private func apply(_ change: PickerModel.Change) {
        switch change {
        case let .open(anchor):
            showPicker(at: anchor)
        case .rows:
            pickerPopover?.reload()
        case .field:
            pickerPopover?.focusField()
        case .close:
            pickerPopover?.close()
        }
    }

    /// Hang the picker from what the controller named: under the cursor's
    /// row at the subject column, or under the open message's button for
    /// the verb (More's, when it has folded away).
    private func showPicker(at anchor: PickerAnchorFfi) {
        guard let picker, let popover = pickerPopover else { return }
        switch anchor {
        case let .row(position):
            guard let table = focusTable else { return }
            table.tableView.scrollRowToVisible(Int(position))
            guard let rect = table.pickerAnchor(row: Int(position), width: PickerMetrics.width)
            else { return }
            popover.show(relativeTo: rect, of: table.tableView)
        case let .result(position):
            // Under the focused result, at its content column (spec 010
            // US5): the rows checked, or that one.
            guard let table = resultsTable, let row = results?.tableRow(of: position) else { return }
            table.tableView.scrollRowToVisible(row)
            let frame = table.tableView.rect(ofRow: row)
            let content = ResultRowView.Columns.leading + ResultRowView.Columns.gutter
                + ResultRowView.Columns.sender + 2 * ResultRowView.Columns.gap
            popover.show(
                relativeTo: NSRect(
                    x: frame.minX + content, y: frame.minY, width: PickerMetrics.width, height: frame.height),
                of: table.tableView)
        case .openMessage:
            guard let content = messageContent, content.window != nil else { return }
            let frame = verbFrames[PickerCommand.opening(picker.kind)]
                ?? verbFrames[PickerCommand.more]
                ?? CGRect(x: 12, y: 0, width: PickerMetrics.width, height: MessageActionRow.height)
            // The frames are SwiftUI's, from the content's top left.
            let rect = content.isFlipped
                ? frame
                : CGRect(x: frame.minX, y: content.bounds.height - frame.maxY, width: frame.width, height: frame.height)
            popover.show(relativeTo: rect, of: content)
        }
    }

    private func makeBar(_ session: PostioSession) {
        let bar = CommandBarModel(engine: session)
        commandBar = bar
        barPanel = CommandBarPanel(model: bar) { [weak session] in
            KeyCapSpelling.cap(session?.binding(for: BarCommand.saveSearch))
        }
        let places = PlacesModel(engine: session)
        self.places = places
        placesPopover = PlacesPopover(model: places) { [weak self] in self?.placesClosed() }
    }

    /// What the controller said about the bar.
    private func apply(_ change: CommandBarModel.Change) {
        switch change {
        case let .open(text, selection):
            guard let field = searchField else { return }
            field.stringValue = text
            keycapsChanged?()
            barShown?(true)
            barPanel?.show(under: field)
            if field.currentEditor() == nil {
                field.window?.makeFirstResponder(field)
            }
            field.currentEditor()?.selectedRange = selection
            // What was typed into the results' chips before the bar was up:
            // the bar's field takes it, as if typed there.
            if !typedAhead.isEmpty, let editor = field.currentEditor() as? NSTextView {
                let typed = typedAhead
                typedAhead = ""
                editor.insertText(typed, replacementRange: editor.selectedRange())
            }
        case .lines:
            barPanel?.relayout()
            // The ghost after the caret, and the operator's face (§2).
            let dropdown = commandBar?.showsDropdown == true ? commandBar?.dropdown : nil
            searchField?.ghost = dropdown?.ghost
            searchField?.operatorTyped = dropdown?.state == .operator
        case .close:
            barPanel?.hide()
            barShown?(false)
            searchField?.ghost = nil
            searchField?.operatorTyped = false
            searchField?.stringValue = ""
            keycapsChanged?()
            typedAhead = ""
        }
    }

    // MARK: the results view (specs/010-focus-search step 3)

    /// The results' query: the field's chips and the filter bar's buttons.
    private(set) var searchQuery: SearchQueryModel?

    /// The results' frame, rows and focus ring.
    private(set) var results: ResultsModel?

    /// The results table, in the inbox's place while the results are up.
    @ObservationIgnored private(set) var resultsTable: ResultsTable?

    /// The Files tab's grid (step 9), in the table's place while it is the
    /// tab shown, and the system's Quick Look or save panel over a file.
    @ObservationIgnored private(set) var filesGrid: FilesGrid?
    @ObservationIgnored private(set) var filePreview: FilePreview?

    /// The filter popover that is up (step 4), and what hangs it from its
    /// button.
    @ObservationIgnored private(set) var filterPopover: FilterPopoverModel?
    @ObservationIgnored private(set) var filterPopoverPresenter: FilterPopover?

    /// Quick Look over the results (step 5), and the panel it is drawn in.
    @ObservationIgnored private(set) var quickLook: QuickLookModel?
    @ObservationIgnored private(set) var quickLookPanel: QuickLookPanel?

    /// The no-results page (step 7), in the rows' place while the results
    /// found nothing.
    private(set) var noResults: NoResultsModel?

    /// The Save popover (step 6), and what hangs it from Save search.
    @ObservationIgnored private(set) var savePopover: SavePopoverModel?
    @ObservationIgnored private(set) var savePresenter: SavePopover?

    /// The results' chrome words, `postio-ui`'s.
    let searchWords = focusSearchWords()

    /// The toolbar: the results' items or the inbox's.
    @ObservationIgnored var resultsShown: ((Bool) -> Void)?

    /// The toolbar's query box, while the results' items are up.
    @ObservationIgnored weak var queryBox: ResultsQueryBox?

    /// Typed into the chips before the bar was up, for its field to take.
    @ObservationIgnored private var typedAhead = ""

    private func makeResults(_ session: PostioSession) {
        searchQuery = SearchQueryModel(engine: session)
        let results = ResultsModel(engine: session)
        self.results = results
        resultsTable = ResultsTable(model: results)
        let grid = FilesGrid(model: results)
        let preview = FilePreview(done: { [weak session] in session?.focusSearchFileDone() })
        grid.preview = preview
        filesGrid = grid
        filePreview = preview
        let popover = FilterPopoverModel(engine: session)
        filterPopover = popover
        filterPopoverPresenter = FilterPopover(model: popover)
        let look = QuickLookModel(engine: session)
        quickLook = look
        quickLookPanel = QuickLookPanel(model: look)
        let save = SavePopoverModel(engine: session)
        savePopover = save
        savePresenter = SavePopover(model: save)
        noResults = NoResultsModel(engine: session)
    }

    /// "5 selected", while results are checked.
    var resultsChecked: String? {
        guard let selected = results?.selected, selected > 0 else { return nil }
        return focusSearchChecked(n: selected)
    }

    /// The query box draws what the last `FocusQuery` said.
    func redrawQuery() {
        guard let query = searchQuery else { return }
        queryBox?.show(chips: query.chips, words: query.words, hint: query.hint, nothingFound: query.ringed)
    }

    /// The query box asks for the dropdown: a click, or `/` in it.
    func queryFieldFocused() {
        guard commandBar?.isOpen != true else { return }
        run(BarCommand.search)
    }

    /// Words typed into the query box's chips: the bar opens on the query
    /// and its field takes them.
    func queryFieldTyped(_ text: String) {
        typedAhead += text
        if let field = searchField, commandBar?.isOpen == true, let editor = field.currentEditor() as? NSTextView {
            let typed = typedAhead
            typedAhead = ""
            editor.insertText(typed, replacementRange: editor.selectedRange())
            return
        }
        run(BarCommand.search)
    }

    /// What the results' coming, going or moving asks of the window.
    private func resultsMoved(_ change: ResultsModel.Change) {
        switch change {
        case .open:
            resultsShown?(true)
            redrawQuery()
            if commandBar?.isOpen != true { resultsTable?.takeKeyboard() }
        case .close:
            resultsShown?(false)
            if let table = focusTable?.tableView, let window = table.window, commandBar?.isOpen != true {
                window.makeFirstResponder(table)
            }
        case .redraw:
            // A tab changed under the keyboard: the grid or the table has
            // it, whichever is shown. After SwiftUI has swapped them in --
            // the view being left takes the keyboard with it, to the next
            // key view (a tab button) -- and never from a field.
            DispatchQueue.main.async { [weak self] in
                guard let self, self.commandBar?.isOpen != true, self.results?.isOpen == true,
                      let window = self.mainWindow, !(window.firstResponder is NSText)
                else { return }
                let responder = window.firstResponder
                if self.results?.isFiles == true, !(responder is FilesCollectionView) {
                    self.filesGrid?.takeKeyboard()
                } else if self.results?.isFiles != true, responder is FilesCollectionView
                    || responder === window
                {
                    self.resultsTable?.takeKeyboard()
                }
            }
        case .rows, .cursor:
            break
        }
    }

    /// The toolbar's field took the keyboard. A click into it opens the
    /// bar, as `/` does; `/` and ⌘K focus it once the bar is up already.
    func searchFieldFocused() {
        guard let bar = commandBar, !bar.isOpen else { return }
        run(BarCommand.search)
    }

    /// The field's words changed.
    func searchFieldTyped(_ text: String) {
        commandBar?.typed(text)
    }

    /// The field gave up the keyboard -- a click outside, Tab past it. If
    /// the bar was up, that closed it, and the controller is told.
    func searchFieldLeft() {
        guard let bar = commandBar, bar.closedByToolkit() else { return }
        barPanel?.hide()
        barShown?(false)
        searchField?.stringValue = ""
        keycapsChanged?()
        session?.focusSurfaceClosed(.bar)
    }

    /// A key the field's editor would act on, while the bar is up: the
    /// arrows walk the lines, Return runs one, Tab steps into the chips
    /// (or is the toolkit's), Escape is Back. `false` leaves it to AppKit.
    func searchFieldCommand(_ selector: Selector) -> Bool {
        guard let bar = commandBar, bar.isOpen else { return false }
        switch selector {
        case #selector(NSResponder.moveUp(_:)):
            bar.move(by: -1)
        case #selector(NSResponder.moveDown(_:)):
            bar.move(by: 1)
        case #selector(NSResponder.insertNewline(_:)):
            // ⌘↩ is the dropdown's Show all (specs/010-focus-search).
            if NSApp.currentEvent?.modifierFlags.contains(.command) == true, bar.showAll() {
                return true
            }
            // ⌥↩ excludes the highlighted person, label or folder (US7).
            if NSApp.currentEvent?.modifierFlags.contains(.option) == true, bar.excludeHighlighted() {
                return true
            }
            bar.runHighlighted()
        case #selector(NSResponder.insertNewlineIgnoringFieldEditor(_:)):
            // What a field editor makes of ⌥↩.
            if bar.excludeHighlighted() { return true }
            bar.runHighlighted()
        case #selector(NSResponder.insertTab(_:)):
            return bar.tab()
        case #selector(NSResponder.deleteWordBackward(_:)):
            // ⌥⌫ on a recent search forgets it; anywhere else it is the
            // field's own.
            return bar.forgetHighlighted()
        case #selector(NSResponder.cancelOperation(_:)):
            bar.back()
        default:
            return false
        }
        return true
    }

    /// What the controller said about the places.
    private func apply(_ change: PlacesModel.Change) {
        switch change {
        case .open:
            placesPopover?.anchor = placesAnchor
            placesPopover?.show()
        case .entries:
            placesPopover?.reload()
        case .place:
            // The strip reads `places.placeName`.
            break
        }
    }

    /// The popover closed: the keyboard goes back to the list, unless what
    /// was opened from it is the bar (a label is its search), which holds
    /// the keyboard now.
    private func placesClosed() {
        guard commandBar?.isOpen != true else { return }
        guard let table = focusTable?.tableView, let window = table.window else { return }
        window.makeFirstResponder(table)
    }

    // MARK: what Postio says back

    /// The key map (`?`, T106): the controller opens and closes it; this
    /// holds what it said for the main window's panel.
    let keyMap = KeyMapModel()

    /// The key map was closed by a click outside it: the controller is told,
    /// and sends the keyboard home.
    func keyMapDismissed() {
        guard keyMap.closedByToolkit() else { return }
        session?.focusSurfaceClosed(.keyMap)
    }

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
        // arm's business. Completions, undos and refusals are the
        // controller's toast in this window; only a failure is a notice.
        if let arriving = Notice(event), arriving.shownBesideFocusToast {
            notice = Notice.winner(showing: notice, arriving: arriving)
            noticeToken += 1
        }
        // The command bar and the folders popover (T085, T086).
        if let change = commandBar?.apply(event) {
            apply(change)
            return
        }
        if let change = places?.apply(event) {
            apply(change)
            return
        }
        // The key map (T106), before the surfaces: its close is
        // `FocusCloseSurface(.keyMap)`. The controller put it on its stack,
        // so its opening is not reported back.
        if keyMap.apply(event) != nil { return }
        // The pickers at the row (T092), before the surfaces: its close is
        // `FocusCloseSurface(.picker)`.
        if let change = picker?.apply(event) {
            apply(change)
            return
        }
        // Filtered (T113), drawn in the list's place: its view, its focus,
        // and its close, `FocusCloseSurface(.filtered)`.
        if filtered?.apply(event) != nil { return }
        // The digest's window (T114): `FocusOpenDigest`, every
        // `FocusDigest`, and `FocusCloseSurface(.digest)`.
        if let change = digest?.apply(event) {
            apply(change)
            return
        }
        // The digest-this-sender sheet (T115): `FocusOpenRule`, every
        // `FocusRule`, and `FocusCloseSurface(.dialog)`.
        if let change = ruleSheet?.apply(event) {
            apply(change)
            return
        }
        // Capture (T116): `FocusOpenCapture`, every `FocusCapture`, and
        // `FocusCloseSurface(.capture)`.
        if let change = capture?.apply(event) {
            apply(change)
            return
        }
        // The composer (T079): `FocusComposer`, `FocusSaveDraft`, and
        // `FocusCloseSurface(.composer)`.
        if let change = composer?.apply(event) {
            apply(change)
            return
        }
        // The results view (specs/010-focus-search step 3): its query, its
        // frame, its pages and its focus ring, and leaving it. The list's
        // own cursor and selection follow `FocusLeaveResults` as intents.
        if case .focusQuery = event {
            searchQuery?.apply(event)
            redrawQuery()
            return
        }
        // A filter popover (step 4): hung from its button, drawn whole, or
        // taken down when the controller says.
        if let change = filterPopover?.apply(event) {
            filterPopoverPresenter?.apply(change)
            return
        }
        // The Save popover (step 6): hung from Save search on ⌘S, taken
        // down when its Save went to the controller.
        if let change = savePopover?.apply(event) {
            savePresenter?.apply(change)
            return
        }
        // Quick Look (step 5): a panel over the results, drawn whole each
        // time and closed when the controller says.
        if let change = quickLook?.apply(event) {
            quickLookPanel?.apply(change, over: mainWindow)
            return
        }
        // The no-results page (step 7): drawn whole each time, gone with a
        // new query or with the results. Leaving the results is also the
        // results' event, so it is told and handed on.
        if case .focusRelaxations = event {
            noResults?.apply(event)
            return
        }
        if case .focusLeaveResults = event { noResults?.apply(event) }
        // A file handed to the system (step 9): its Quick Look, from the
        // grid that drives it, or a save panel.
        if case let .focusFileCopy(copy) = event {
            if copy?.save == false { filesGrid?.takeKeyboard() }
            filePreview?.apply(copy, over: mainWindow)
            return
        }
        if let change = results?.apply(event) {
            if change == .close { searchQuery?.apply(event) }
            resultsTable?.apply(change)
            filesGrid?.apply(change)
            resultsMoved(change)
            return
        }
        // The controller's intents: the cursor, the selection, `!`'s heading,
        // the toast (T049). The table draws what changed.
        if let change = focus.apply(event) {
            focusTable?.apply(change)
            // Every verb and every undo says a toast, and either may have
            // moved the stack's top.
            if change == .toast { refreshUndo() }
            return
        }
        // The banner, the sync label and the empty page (T099): the
        // controller's words, held for the strip, the toolbar and the list.
        if states.apply(event) != nil { return }
        if let surface = FocusIntents.surface(event) {
            apply(surface)
            return
        }
        switch event {
        case let .focusListChanged(total):
            focusTable?.listChanged(total: total)
            refreshCounts()
        case let .focusPageReady(page):
            focusTable?.pageArrived(page)
            listTakesTheKeyboard()
            replayDemoKeys()
            if !listLanded {
                listLanded = true
                for route in waitingLinks.take() { follow(route) }
            }
        case .keymapChanged:
            keymapVersion += 1
            focusTable?.keymapChanged()
            keycapsChanged?()
            // The menus show the new keys (T105), and an open key map draws
            // them (T106): the session has the new keymap by now.
            if menuPlan.apply(event) { mountMenuBar() }
            if keyMap.isOpen, let session { keyMap.refresh(session.focusKeyMap()) }
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
        case let .focusConfirm(confirm):
            ask(ConfirmQuestion(confirm))
        case let .focusRun(command):
            // A line of the bar the controller hands back: Compose,
            // Settings, a host verb -- run as a menu item would run it.
            run(command)
        default:
            break
        }
    }

    /// Whether the list has been given the keyboard since the session
    /// opened.
    @ObservationIgnored
    private var listHadKeyboard = false

    /// Once, when the list's first page lands: the table takes the keyboard
    /// (C30, the cursor's row is where the keys go). Left to AppKit, the
    /// window's first key view had it -- Inbox ▾, ringed, which Space
    /// would press -- until a surface sent the keyboard home.
    private func listTakesTheKeyboard() {
        guard !listHadKeyboard, let table = focusTable?.tableView, let window = table.window else { return }
        listHadKeyboard = true
        guard commandBar?.isOpen != true, picker?.isOpen != true,
              !(window.firstResponder is NSTextView)
        else { return }
        window.makeFirstResponder(table)
    }

    /// The keys a demo was asked to press (`DemoMode.keys`), until they
    /// have been: a screen that needs a state is photographed in it.
    @ObservationIgnored
    private var demoKeys = DemoMode.keys

    /// Press the demo's keys on the list, once, after its first page has
    /// landed, each through the resolver and `run` as a real press would
    /// go, with a pause for what it opened to land.
    private func replayDemoKeys() {
        guard !demoKeys.isEmpty || DemoMode.snapshot != nil, !demoReplayed else { return }
        demoReplayed = true
        let keys = demoKeys
        demoKeys = []
        Task { @MainActor [weak self] in
            defer { self?.drawDemoSnapshot() }
            for key in keys {
                try? await Task.sleep(nanoseconds: 400_000_000)
                guard let self, let session = self.session else { return }
                if self.replayClick(key) { continue }
                if self.replayIntoField(key) { continue }
                // With the bar up, a chord is the search field's, as a press
                // there resolves it: ⌘↩ is Show all, ⌥⌫ forgets a recent.
                let searching = self.commandBar?.isOpen == true
                if case let .command(id) = session.key(
                    key, in: searching ? .search : .list, typing: searching)
                {
                    self.run(id)
                }
            }
        }
    }

    @ObservationIgnored private var demoReplayed = false

    /// Draw the main window, its toolbar and its children to
    /// `DemoMode.snapshot`, once what the keys opened has landed.
    private func drawDemoSnapshot() {
        guard let path = DemoMode.snapshot else { return }
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            guard let window = self?.mainWindow, let frame = window.contentView?.superview else { return }
            frame.layoutSubtreeIfNeeded()
            guard let rep = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) else { return }
            frame.cacheDisplay(in: frame.bounds, to: rep)
            // The windows over it -- a filter popover, the bar's panel --
            // are windows of their own, so a picture of the main window's
            // views alone leaves them out: each is drawn in at its place.
            let over = NSApp.windows.filter { other in
                other !== window && other.isVisible && other.frame.intersects(window.frame)
                    && other.level.rawValue >= window.level.rawValue
            }
            let image = NSImage(size: frame.bounds.size)
            image.lockFocus()
            rep.draw(in: NSRect(origin: .zero, size: frame.bounds.size))
            for other in over.sorted(by: { $0.orderedIndex > $1.orderedIndex }) {
                guard let view = other.contentView?.superview,
                      let piece = view.bitmapImageRepForCachingDisplay(in: view.bounds)
                else { continue }
                view.cacheDisplay(in: view.bounds, to: piece)
                let origin = NSPoint(
                    x: other.frame.minX - window.frame.minX, y: other.frame.minY - window.frame.minY)
                piece.draw(
                    in: NSRect(origin: origin, size: other.frame.size), from: .zero, operation: .sourceOver,
                    fraction: 1, respectFlipped: true, hints: nil)
            }
            image.unlockFocus()
            guard let tiff = image.tiffRepresentation, let whole = NSBitmapImageRep(data: tiff) else { return }
            try? whole.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
        }
    }

    /// A demo's click on a filter button, named `@from`, `@to`, `@date`,
    /// `@anywhere` or `@label` (screens 08 and 09): the format of
    /// `POSTIO_DEMO_KEYS` has no pointer, and a popover opens from a click.
    /// `false` for any other key.
    private func replayClick(_ key: KeyEvent.Reduced) -> Bool {
        guard key.name == nil, let word = key.character, word.hasPrefix("@"), word.count > 1 else { return false }
        let kinds: [String: FilterKindFfi] = [
            "@from": .from, "@to": .to, "@date": .date, "@anywhere": .anywhere, "@label": .label,
        ]
        guard let kind = kinds[word.lowercased()] else { return false }
        searchQuery?.tap(kind)
        return true
    }

    /// A replayed key, given to the bar's or the popover's field while one
    /// is up, as a press there would be: words typed, the arrows, Return,
    /// Tab, Escape. `false` for a key the resolver should have.
    private func replayIntoField(_ key: KeyEvent.Reduced) -> Bool {
        let plain = !key.modifiers.command && !key.modifiers.control && !key.modifiers.option
        guard plain else { return false }
        if let picker, picker.isOpen, let popover = pickerPopover {
            switch key.name {
            case "down": picker.move(by: 1)
            case "up": picker.move(by: -1)
            case nil:
                // Typing, where a press would type: into the date field once
                // it has the keyboard, or into the filter -- except a bare
                // digit or space while the filter is empty, which is the
                // picker's, as the resolver has it.
                let text = key.character ?? ""
                let picks = text == " " || (text.count == 1 && text.allSatisfy(\.isNumber))
                let typing = picker.inField || (picker.filters && !(picker.typed.isEmpty && picks))
                guard typing else { return false }
                popover.type(text)
            // Return, Tab and Escape resolve as a press does.
            default: return false
            }
            return true
        }
        if let model = filterPopover, let view = model.view {
            switch key.name {
            case "down": model.moveHighlight(by: 1)
            case "up": model.moveHighlight(by: -1)
            case "return": model.apply()
            case "escape": model.cancel()
            case nil:
                let text = key.character ?? ""
                if view.kind == .date {
                    model.words(model.wordsText + text)
                } else if text == " ", model.filterText.isEmpty {
                    // Space toggles the highlighted row, as in the popover.
                    model.toggleHighlighted()
                } else {
                    model.filter(model.filterText + text)
                }
            default: return false
            }
            return true
        }
        if let popover = placesPopover, popover.isShown {
            switch key.name {
            case "return": popover.openHighlighted()
            case "down": places?.move(by: 1)
            case "up": places?.move(by: -1)
            case "escape": popover.close()
            case nil: popover.type(key.character ?? "")
            default: return false
            }
            return true
        }
        // A composer the demo opened: characters are typed into To, where
        // a new message has the keyboard, so screen 05's list can be shown.
        if let composer, composer.isOpen, let model = composer.model, key.name == nil {
            model.to += key.character ?? ""
            return true
        }
        if let bar = commandBar, bar.isOpen, let field = searchField {
            switch key.name {
            case "return": bar.runHighlighted()
            case "down": bar.move(by: 1)
            case "up": bar.move(by: -1)
            case "tab": _ = bar.tab()
            case "escape": run(BarCommand.back)
            case nil:
                // Through the field's editor, as a key press types: the
                // caret moves on, and the field says it changed.
                if let editor = field.currentEditor() as? NSTextView {
                    editor.insertText(key.character ?? "", replacementRange: editor.selectedRange())
                } else {
                    field.stringValue += key.character ?? ""
                    searchFieldTyped(field.stringValue)
                    keycapsChanged?()
                }
            default: return false
            }
            return true
        }
        return false
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

    /// Focus's menu bar, planned from the registry and the bindings in
    /// force (T105): asked of the session once there is one, and planned
    /// again on `KeymapChanged`, by which time `bindingsFor` answers the
    /// new keys.
    @ObservationIgnored
    private lazy var menuPlan = MenuBarPlan { [weak self] command in
        self?.session?.bindings(for: command) ?? []
    }

    /// Plan the menu bar from the bindings in force and hang it off `NSApp`
    /// (#657). Accelerators come from the session's keymap where there is a
    /// session to ask, and are absent before; dispatch is the key monitor's.
    private func installMenuBar() {
        menuPlan.rebuild()
        mountMenuBar()
    }

    private func mountMenuBar() {
        MenuBar.install(
            menus: { [weak self] in self?.menuPlan.menus ?? [] },
            available: { [weak self] id in
                guard let self else { return false }
                guard let session else {
                    // Before a session, the verbs this frontend handles itself
                    // still work: Settings edits a file.
                    return Intercepted.all.contains(id)
                }
                return session.isAvailable(id, in: self.context)
            },
            run: { [weak self] id in self?.run(id) },
            undo: undoRouter
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
        // The banner's password sheet has the keyboard: every key reaches
        // its field and its buttons -- Return saves, Escape cancels --
        // rather than the list behind it.
        if bannerRepair.asking != nil { return false }
        // A question's alert is up: Return and Escape are its buttons'.
        if asking != nil { return false }
        // The Save popover has the keyboard: Return saves, Escape cancels,
        // every other key is its field's and its switches'.
        if let save = savePopover, save.isOpen {
            switch id {
            case Intercepted.back: save.cancel()
            case "open_message", "picker_confirm": save.save()
            default: return false
            }
            return true
        }
        // The rule sheet holds the keyboard (a dialog takes every key in
        // the controller, which answers only Back): Escape is Back, which
        // closes it; Return is Create; every other key is the sheet's
        // fields' and menus'.
        if let sheet = ruleSheet, sheet.isOpen {
            switch id {
            case Intercepted.back: session?.invoke(id)
            case "open_message", "picker_confirm": sheet.create()
            default: return false
            }
            return true
        }
        // Space and Return in a picker are about the highlighted row, which
        // only the popover knows; Escape is its Back, before any surface of
        // the Mac's own is asked.
        if let picker, picker.isOpen {
            if picker.run(id) { return true }
            if id == Intercepted.back {
                picker.back()
                return true
            }
        }
        switch id {
        case Intercepted.back where keyWindow.current == .compose && composer?.model?.suggesting != nil:
            // The recipient list first: Escape takes it down and leaves the
            // words; the next Escape closes the composer.
            composer?.model?.dismissSuggestions()
        case Intercepted.back where keyWindow.current == .message && messageWindow?.showingSource == true:
            // Esc from the raw source returns to the message (M4); the
            // controller does not know the source is up.
            messageWindow?.closeSource()
        case Intercepted.back where placesPopover?.isShown == true:
            // The folders popover is not a surface the controller keeps:
            // Escape closes it here, and the keyboard goes home.
            placesPopover?.close()
        case Intercepted.settings:
            // A request the main window turns into `openWindow(id:)`,
            // because only a view can open a window (#1261).
            settingsWindow.raise()
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
        case Intercepted.updateCredential where keyWindow.current == .main && states.banner?.account != nil:
            // The sign-in banner's button (screen 19): the account it names,
            // by the route the account calls for -- a password sheet, or
            // the browser for an OAuth grant.
            guard let id = states.banner?.account,
                  let account = (session?.accounts() ?? accounts).first(where: { $0.id == id })
            else { return false }
            let session = session
            Task { await bannerRepair.begin(account, through: session) }
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
               let composer = composer?.model,
               ComposeCommands.run(id, on: composer, through: session)
            {
                // ⌘↩ queued it: the window closes, as Send's button closes it.
                if composer.sent { secondary.close(.composer) }
                return true
            }
            session?.invoke(id)
        }
        return true
    }

    // MARK: postio:// links (T117)

    /// Links that came before the list had landed, opened once it has: a
    /// cold launch from a captured line has no session to ask until then.
    @ObservationIgnored
    private var waitingLinks = PostioLink.Waiting()

    /// Whether the list's first page has landed, so a link can be opened.
    @ObservationIgnored
    private var listLanded = false

    /// Follow a `postio://` link: the controller opens the message it names
    /// in the message window (or says it is gone); any other link of
    /// Postio's is said in the pill at once. The main window comes forward
    /// first, since the link was clicked in another app.
    func follow(_ route: PostioLink.Route) {
        guard let session, listLanded else {
            waitingLinks.hold(route)
            return
        }
        mainWindow?.makeKeyAndOrderFront(nil)
        switch route {
        case let .open(uri, _):
            session.focusOpenLink(uri: uri)
        case let .unknown(words):
            if focus.apply(PostioLink.toast(words)) != nil { refreshUndo() }
        }
    }

    /// Open a composer on a `mailto:` link. `false` when there is nothing to
    /// open one from, which the caller says out loud.
    func write(mailto: Mailto) -> Bool {
        guard let session, let composer, let draft = session.mailtoDraft(mailto) else { return false }
        // The Mac made this draft, so the controller is told the composer
        // is up (it ends the composition the composer held first).
        composer.open(own: draft)
        showComposerWindow()
        return true
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
