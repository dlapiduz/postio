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
            messageWindow = MessageWindowModel(source: session) { [weak session] more, finding in
                session?.focusReaderState(moreOpen: more, finding: finding)
            }
            makeBar(session)
            makePicker(session)
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
    var mainContext: UiContext { commandBar?.isOpen == true ? .search : .list }

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

    /// The message under the Focus cursor, if its row has arrived.
    var focusCursorMessage: Int64? {
        guard let table = focusTable, let row = table.cursor else { return nil }
        return table.model.row(at: row)?.id
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

    /// The message the open message window shows, while it has the
    /// keyboard: what Reply, Reply all and Forward answer from it.
    private var messageInFront: Int64? {
        keyWindow.current == .message ? messageWindow?.shown : nil
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
        case .keyboardHome:
            guard let table = focusTable?.tableView, let window = table.window else { return }
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(table)
        case let .openDraft(message):
            write(session?.draftForMessage(message))
        case .openDigest:
            // The digest's window is a later phase (spec 009 US9). The kind
            // only: a log never carries which delivery, let alone what it holds.
            Self.log.info("a digest window was asked for; not built on the Mac yet")
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
            barPanel?.show(under: field)
            if field.currentEditor() == nil {
                field.window?.makeFirstResponder(field)
            }
            field.currentEditor()?.selectedRange = selection
        case .lines:
            barPanel?.relayout()
        case .close:
            barPanel?.hide()
            searchField?.stringValue = ""
            keycapsChanged?()
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
            bar.runHighlighted()
        case #selector(NSResponder.insertTab(_:)):
            return bar.tab()
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
        // The pickers at the row (T092), before the surfaces: its close is
        // `FocusCloseSurface(.picker)`.
        if let change = picker?.apply(event) {
            apply(change)
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
        case let .focusRun(command):
            // A line of the bar the controller hands back: Compose,
            // Settings, a host verb -- run as a menu item would run it.
            run(command)
        case .focusShowFiltered:
            // The Filtered view is a later phase (spec 009 T113).
            Self.log.info("the Filtered view was asked for; not built on the Mac yet")
            notice = Notice(kind: .refused, message: "Filtered is not built on the Mac yet.", undoable: false)
            noticeToken += 1
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
        guard !demoKeys.isEmpty else { return }
        let keys = demoKeys
        demoKeys = []
        Task { @MainActor [weak self] in
            for key in keys {
                try? await Task.sleep(nanoseconds: 400_000_000)
                guard let self, let session = self.session else { return }
                if self.replayIntoField(key) { continue }
                if case let .command(id) = session.key(key, in: .list, typing: false) {
                    self.run(id)
                }
            }
        }
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
        case Intercepted.cheatSheet:
            showingCheatSheet = true
        case Intercepted.back where keyWindow.current == .message && messageWindow?.showingSource == true:
            // Esc from the raw source returns to the message (M4); the
            // controller does not know the source is up.
            messageWindow?.closeSource()
        case Intercepted.back where placesPopover?.isShown == true:
            // The folders popover is not a surface the controller keeps:
            // Escape closes it here, and the keyboard goes home.
            placesPopover?.close()
        case Intercepted.back where keyWindow.current == .main && showingCheatSheet:
            showingCheatSheet = false
        case Intercepted.settings:
            // A request the main window turns into `openWindow(id:)`,
            // because only a view can open a window (#1261).
            settingsWindow.raise()
        case Intercepted.compose:
            write(session?.newDraft())
        case Intercepted.reply:
            write(replyDraft(all: false, to: target ?? messageInFront))
        case Intercepted.replyAll:
            write(replyDraft(all: true, to: target ?? messageInFront))
        case Intercepted.forward:
            guard let session, let message = target ?? messageInFront ?? focusCursorMessage
            else { return false }
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
        // One secondary window at a time (M4): the composer replaces the
        // message it answers. The composer is not yet a surface the
        // controller is told of (phase 4), so it is closed here.
        secondary.close(.message)
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
