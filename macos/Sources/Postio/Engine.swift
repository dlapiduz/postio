import AppKit
import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI
import os

/// Holds the session or the reason the store would not open. An ad-hoc-signed build has a new
/// code identity on every rebuild, so the Keychain asks again each time and the app must render
/// that.
// `@MainActor`: everything it holds is, the table and the views that read it.
@MainActor
@Observable
final class Engine {
    enum State {
        case opening
        case open
        case refused(StoreRefusalModel)
    }

    private(set) var state: State = .opening

    /// `nil` until a session opens, when the window follows the system.
    private(set) var appearance: AppearanceFfi?

    private(set) var accounts: [AccountFfi] = []
    /// Made once per session, not per draw, so typed text survives redraws.
    private(set) var firstRun: FirstRunModel?

    var colorScheme: ColorScheme? {
        switch appearance?.theme {
        case .light: return .light
        case .dark: return .dark
        case .system, .none: return nil
        }
    }
    private(set) var session: PostioSession?

    /// Opens the store off this actor: the key comes from the login Keychain, which can wait on
    /// a user prompt, and `Engine()` runs inside `App.init()` before SwiftUI has a scene.
    /// Synchronously, the app showed in the Dock with no window (#1146).
    init() {
        // First, so a Keychain or migration refusal is logged rather than an empty window.
        PostioSession.startLogging()
        // Before the store: a bar installed after it was AppKit's stock one for the whole Keychain
        // wait (#1262).
        installMenuBar()
        openStore()
    }

    private func openStore(after started: StartedOverFfi? = nil) {
        state = .opening
        Task.detached(priority: .userInitiated) {
            let opened = Result {
                try DemoMode.seed.map(PostioSession.openDemo) ?? PostioSession.open()
            }
            await MainActor.run { [weak self] in
                self?.adopt(opened)
                if let started, let self, case .open = self.state {
                    self.notice = Notice(
                        kind: .completed, message: startedOverWords(setAside: started.setAside),
                        undoable: false)
                    self.noticeToken += 1
                }
            }
        }
    }

    private func adopt(_ opened: Result<PostioSession, Error>) {
        switch opened {
        case let .success(session):
            self.session = session
            appearance = session.appearance()
            accounts = session.accounts()
            if accounts.isEmpty { firstRun = FirstRunModel(session: session) }
            vouch()
            focusTable = makeFocusTable(session)
            messageWindow = MessageWindowModel(
                source: session,
                report: { [weak session] more, finding in
                    session?.focusReaderState(moreOpen: more, finding: finding)
                },
                markRead: { [weak session] message in session?.markReadOnDwell(message) },
                matchEdge: { [weak session] forward in session?.focusMatchEdge(forward: forward) },
                handOff: { file in
                    // POSTIO-CONSENT: runs only when a person clicked an
                    // attachment's chip or chose it or a link from the menu
                    // `A` offers, one at a time; nothing opens on render, on
                    // arrival or from a setting.
                    NSWorkspace.shared.open(file)
                })
            makeBar(session)
            makePicker(session)
            filtered = FilteredModel(engine: session)
            makeResults(session)
            digest = DigestModel(engine: session, source: session)
            ruleSheet = RuleSheetModel(engine: session)
            capture = CaptureModel(engine: session)
            makeContactPopover(session)
            makeThreadPopovers(session)
            contactsView = ContactsModel(engine: session)
            contactsSettings = ContactsSettingsModel(
                engine: session,
                makeBook: {
                    // The real store is constructed here, when the person
                    // presses Turn on and preview, and never in a demo.
                    DemoMode.seed != nil ? Self.demoBook() : AppleAddressBook()
                })
            contactsIo = ContactsIoModel(
                engine: session, panels: ContactsPanels(), demo: DemoMode.seed != nil)
            composer = ComposerWindow(engine: session)
            keycapsChanged?()
            state = .open
            mailboxes = session.mailboxes
            // Nothing is fetched until an engine starts (#648). A demo never syncs: its account has
            // no server.
            if DemoMode.seed == nil { _ = try? session.startSyncing() }
            if let said = DemoMode.state { _ = session.demoState(said) }
            for repair in [accountRepair, bannerRepair] {
                repair.repaired = { [weak self] account in
                    self?.settingsActions.credentialRepaired(account)
                }
            }
            // An added account gets its engine here, so it syncs without a relaunch (#1299).
            settingsActions.accountAdded = { [weak self] in
                guard let self, let session = self.session else { return }
                self.accounts = session.accounts()
                if !self.accounts.isEmpty { self.firstRun = nil }
                self.vouch()
                _ = try? session.startSyncing()
                self.mailboxes = session.mailboxes
            }
            notifications.start()
            notifications.open = { _, _ in }
            consumeEvents(from: session)
            NotificationCenter.default.addObserver(
                forName: NSWindow.didBecomeKeyNotification, object: nil, queue: .main
            ) { [weak self] note in
                guard let window = note.object as? NSWindow else { return }
                MainActor.assumeIsolated {
                    guard KeyWindowTracker.isMain(window) else { return }
                    self?.refreshUndo()
                }
            }
            // The read clock runs only while Postio is the active app.
            messageWindow?.setAppActive(NSApplication.shared.isActive)
            for (name, active) in [
                (NSApplication.didBecomeActiveNotification, true),
                (NSApplication.didResignActiveNotification, false),
            ] {
                NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) {
                    [weak self] _ in
                    MainActor.assumeIsolated { self?.messageWindow?.setAppActive(active) }
                }
            }
            refreshUndo()
            openFocus(.inbox)
            // Keystrokes, resolved by the core (#656), only once there is a
            // keymap to ask: a monitor that swallowed keys to answer nothing
            // would make the unavailable screen unusable as well as empty.
            installKeyboard(session)
            installMenuBar()
            // The boundary absorbs repeats; no debounce here.
            reachability.start { [weak self] offline in
                Task { @MainActor in
                    self?.session?.setOffline(offline)
                    self?.vouch()
                    if !offline { self?.messageWindow?.networkReturned() }
                }
            }
        case let .failure(error):
            // The boundary's sentence, not one invented here. Logged by case only: it may name a
            // path.
            Self.log.error("the store did not open")
            let words = storeRefusalWords()
            session = nil
            state = .refused(
                StoreRefusalModel(
                    refusal: StoreRefusal(error, words: words), words: words,
                    startOver: { try PostioFFI.startOver(storePath: nil) },
                    reopen: { [weak self] started in self?.openStore(after: started) }))
        }
    }

    private(set) var mailboxes: [MailboxFfi] = []

    private(set) var settingsWindow = WindowRequest(id: WindowId.settings)

    private(set) var composer: ComposerWindow?

    /// Lent to recipient completion after one prompt; read per answer, kept nowhere.
    @ObservationIgnored
    private lazy var contacts = ContactsSource(book: SystemContactBook())

    static let composerWidth: CGFloat = 980

    private func apply(_ change: ComposerWindow.Change) {
        switch change {
        case let .open(kind, message):
            openComposer(kind, answering: message)
        case let .save(composition):
            guard let session else { return }
            composer?.save(composition) { session.saveDraft($0) }
        case .close:
            secondary.close(.composer)
        case .threadChanged:
            break
        }
    }

    private func openComposer(_ kind: ComposerKindFfi, answering message: Int64?) {
        guard let session, let composer else { return }
        let draft: DraftFfi?
        switch kind {
        case .new: draft = session.newDraft()
        case .reply: draft = message.flatMap { session.replyDraft(to: $0, all: false) }
        case .replyAll: draft = message.flatMap { session.replyDraft(to: $0, all: true) }
        case .forward: draft = message.flatMap { session.forwardDraft($0) }
        case .draft: draft = message.flatMap { session.draftForMessage($0) }
        // An unsent reply in a thread has no message: `message` is its id.
        case .threadDraft: draft = message.flatMap { session.draft($0) }
        }
        guard let draft else {
            NSSound.beep()
            composer.couldNotOpen()
            return
        }
        composer.show(draft)
        showComposerWindow()
    }

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
        if let window = secondary.window, secondary.kind == .composer {
            chrome.install(on: window)
        }
    }

    @ObservationIgnored private var composeChrome: ComposeWindowChrome?

    /// What the settings window's account actions are doing, held here
    /// because their progress arrives as events and a window that owned
    /// them would have to be open at the moment one landed.
    let settingsActions = AccountActions()

    let settingsAccounts = SettingsAccounts()

    /// Putting a broken account back in service. Held here because
    /// `update_credential` is a command, and a command cannot reach a view.
    let accountRepair = AccountRepair()

    /// Its own, because both windows present whenever theirs is asking.
    let bannerRepair = AccountRepair()

    var bannerRepairAccount: AccountFfi? {
        guard let id = bannerRepair.asking else { return nil }
        return accounts.first { $0.id == id }
    }

    @ObservationIgnored
    private lazy var undoRouter = UndoRouter(
        manager: PostioUndoManager { [weak self] in
            self?.session?.invoke(Notice.undoCommand)
        })

    private func refreshUndo() {
        guard let session else { return }
        let manager = undoRouter.manager
        Task { await manager.refresh { session.undoDescription() } }
    }

    private let notifications = MailNotifications()
    private let reachability = Reachability()
    private var keys: KeyMonitor?

    /// The key monitor sees every key press in the app, compose windows included.
    let keyWindow = KeyWindowTracker()

    var mainContext: UiContext {
        if commandBar?.isOpen == true { return .search }
        if results?.isOpen == true { return .results }
        if contactPopover?.isOpen == true { return .contactPopover }
        if contactsView?.isOpen == true { return .contacts }
        if capture?.isOpen == true { return .capture }
        if digest?.isOpen == true { return .digest }
        if filtered?.isOpen == true { return .filtered }
        return .list
    }

    var context: UiContext {
        KeyboardContext.resolving(keyWindow: keyWindow.current, mainWindow: mainContext)
    }

    private(set) var focusTable: FocusListTable?

    private(set) var focusScope: FocusScopeFfi = .inbox

    private(set) var focusStrip: FocusStripFfi?

    private(set) var keymapVersion = 0

    let focus = FocusIntents()

    private func makeFocusTable(_ session: PostioSession) -> FocusListTable {
        let table = FocusListTable(
            model: FocusListModel(source: session, focus: focus) { [weak session] command in
                session?.binding(for: command)
            })
        // The pointer is told to the controller; the ring and the boxes move
        // when its intents come back, never here.
        table.onPoint = { [weak session] row in session?.focusPoint(row) }
        table.onActivate = { [weak session] row in session?.focusActivate(row) }
        table.onChevron = { [weak session] row in session?.focusChevronClicked(UInt32(max(0, row))) }
        table.onPick = { [weak session] row, range in session?.focusPick(row, range: range) }
        table.onContactName = { [weak self, weak session] row, address in
            self?.contactRow = row
            session?.focusContactPopoverAt(address)
        }
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

    var actionBarWords: ActionBarWords? {
        _ = keymapVersion
        // `vault: false`: the boundary does not say yet whether capture has
        // a vault to write to (C9), and a Task button that can only fail is
        // worse than none.
        return ActionBarWords(
            summary: focus.summary, hasSelection: focus.hasSelection, vault: false
        ) { [weak self] command in self?.session?.binding(for: command) }
    }

    var barHeightUnderToast: CGFloat {
        if filtered?.isOpen == true { return filtered?.bar == nil ? 0 : ActionBar.height }
        return actionBarWords == nil ? 0 : ActionBar.height
    }

    var undoCap: String? {
        _ = keymapVersion
        return KeyCapSpelling.cap(session?.binding(for: Notice.undoCommand))
    }

    var stripWords: HeaderStripWords {
        _ = keymapVersion
        // On while the controller's `!` heading stands (`toggle_has_action`).
        return HeaderStripWords(
            strip: focusStrip, place: places?.placeName ?? "Inbox", hasActionOn: focus.heading != nil
        ) {
            [weak self] command in self?.session?.binding(for: command)
        }
    }

    func openFocus(_ scope: FocusScopeFfi) {
        guard let session else { return }
        focusScope = scope
        session.openFocus(scope)
        refreshCounts()
    }

    /// Events come in bursts during a sync; one read per burst is enough.
    private var countsReading = false
    private var countsOwed = false

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
            notifications.unread(focusStrip?.unread, badge: session.notices().badge)
            if countsOwed {
                countsOwed = false
                refreshCounts()
            }
        }
    }

    private(set) var messageWindow: MessageWindowModel?

    @ObservationIgnored
    private lazy var secondary = SecondaryWindowController { [weak self] kind in
        self?.secondaryClosed(kind)
    }

    @ObservationIgnored
    private var messageChrome: MessageWindowChrome?

    /// Whether the engine has been told the message window is open, so a
    /// close is reported only for an open that was.
    @ObservationIgnored
    private var messageReported = false

    private static let log = Logger(subsystem: "dev.postio.Postio", category: "focus")

    private var mainWindow: NSWindow? {
        focusTable?.tableView.window ?? NSApp.windows.first { KeyWindowTracker.isMain($0) }
    }

    /// Main window in front: Open Filtered draws in the list's place, which settings would hide.
    func runFromSettings(_ command: String) {
        mainWindow?.makeKeyAndOrderFront(nil)
        run(command)
    }

    private func apply(_ surface: FocusIntents.Surface) {
        switch surface {
        case let .openMessage(message, index, total):
            openMessage(message, index: index, total: total)
        case let .close(kind):
            if kind == .message, secondary.kind != .message {
                messageWindow?.closed()
                if messageReported { session?.focusSurfaceClosed(.message) }
                messageReported = false
            } else {
                secondary.close(kind)
            }
        case let .reader(verb):
            messageWindow?.apply(verb)
        case let .thread(said):
            messageWindow?.said(said)
        case .keyboardHome where results?.isOpen == true:
            mainWindow?.makeKeyAndOrderFront(nil)
            if results?.isFiles == true { filesGrid?.takeKeyboard() } else { resultsTable?.takeKeyboard() }
        case .keyboardHome:
            guard let table = focusTable?.tableView, let window = table.window else { return }
            window.makeKeyAndOrderFront(nil)
            window.makeFirstResponder(table)
        case .openDigest:
            break
        }
    }

    private func openMessage(_ message: Int64, index: UInt32, total: UInt32) {
        guard let model = messageWindow, let session, let main = mainWindow else { return }
        notifications.opened(message)
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

    private var attachmentMenu: AttachmentMenu?

    private func presentAttachmentMenu(_ choices: OpenChoicesFfi) {
        guard let window = secondary.window, let view = window.contentView else { return }
        let menu = AttachmentMenu { [weak self, weak window] choice in
            guard let self, let model = self.messageWindow else { return }
            switch choice {
            case let .link(target):
                model.openLink(target)
            case let .open(id):
                model.openAttachment(id)
            case let .save(id, name):
                // POSTIO-CONSENT: a file is written only to the name and
                // place somebody chose in this save panel.
                let panel = NSSavePanel()
                panel.nameFieldStringValue = name
                self.ask(panel, over: window) { model.saveAttachment(id, to: $0) }
            case .saveAll:
                // POSTIO-CONSENT: files are written only into the folder
                // somebody chose in this panel.
                let panel = NSOpenPanel()
                panel.canChooseFiles = false
                panel.canChooseDirectories = true
                panel.canCreateDirectories = true
                panel.allowsMultipleSelection = false
                panel.prompt = choices.saveAll
                self.ask(panel, over: window) { model.saveAttachments(to: $0) }
            }
        }
        attachmentMenu = menu
        menu.present(choices, in: view)
    }

    /// Dismissed is an answer: nothing is written and nothing is said.
    private func ask(_ panel: NSSavePanel, over window: NSWindow?, then: @escaping (URL) -> Void) {
        let answered: (NSApplication.ModalResponse) -> Void = { [weak panel] response in
            guard response == .OK, let url = panel?.url else { return }
            then(url)
        }
        if let window {
            panel.beginSheetModal(for: window, completionHandler: answered)
        } else {
            answered(panel.runModal())
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
            verbFrames: { [weak self] in self?.verbFrames = $0 },
            barRoom: { [weak session] in session?.focusBarRoom($0) },
            barChip: { [weak session] in session?.focusThreadBarChipClicked($0) },
            barMore: { [weak session] in session?.focusThreadBarMoreClicked(from: $0) }
        ) { [weak session, weak model] message, document, card in
            MessageBodyView(
                message: message,
                document: document,
                sentence: card?.sentence,
                find: model?.find.request,
                onFound: { model?.found($0) },
                resolveCid: { session?.resolveCid(message: $0, contentId: $1) },
                resolveFont: { session?.readerFont($0) })
        }
        .preferredColorScheme(colorScheme)
        let hosting = NSHostingView(rootView: content)
        // The window's size is the geometry's (M1), never the content's: a hosting view left to
        // size it grows it to the column.
        hosting.sizingOptions = []
        messageContent = hosting
        secondary.show(
            .message, content: hosting,
            width: CGFloat(document.windowWidth), title: model.view?.subject ?? "", over: main,
            configure: { chrome.install(on: $0) })
        messageReported = true
        session.focusSurfaceOpened(.message)
    }

    private func secondaryClosed(_ kind: SurfaceKindFfi) {
        if kind == .capture {
            if capture?.closedByToolkit() == true { session?.focusSurfaceClosed(.capture) }
            return
        }
        if kind == .composer {
            composeChrome = nil
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

    /// The words are `postio_ui::focus_state`'s; nothing here composes them.
    let states = FocusStates()

    var isOffline: Bool { session?.isOffline ?? false }

    var keycapsChanged: (() -> Void)?

    var barShown: ((Bool) -> Void)?

    private(set) var commandBar: CommandBarModel?
    private(set) var places: PlacesModel?
    @ObservationIgnored private var barPanel: CommandBarPanel?
    @ObservationIgnored private var placesPopover: PlacesPopover?
    @ObservationIgnored weak var searchField: BarSearchField?
    @ObservationIgnored weak var placesAnchor: NSView?

    private(set) var filtered: FilteredModel?

    private(set) var digest: DigestModel?
    @ObservationIgnored private var digestChrome: DigestWindowChrome?

    private func apply(_ change: DigestModel.Change) {
        switch change {
        case .open:
            showDigest()
        case .redraw:
            if let title = digest?.view?.title, secondary.kind == .digest {
                secondary.window?.title = title
            }
        case .close:
            secondary.close(.digest)
        }
    }

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
        hosting.sizingOptions = []
        secondary.show(
            .digest, content: hosting, width: CGFloat(geometry.windowWidth),
            title: model.view?.title ?? "", over: main,
            configure: { chrome.install(on: $0) })
    }

    private(set) var ruleSheet: RuleSheetModel?

    @ObservationIgnored
    private lazy var ruleSheetWindow = FocusSheet { [weak self] in
        guard let self, self.ruleSheet?.closedByToolkit() == true else { return }
        self.session?.focusSurfaceClosed(.dialog)
    }

    private func apply(_ change: RuleSheetModel.Change) {
        switch change {
        case .open:
            guard let model = ruleSheet else { return }
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

    @ObservationIgnored private(set) var contactsSettings: ContactsSettingsModel?

    let settingsSection = SettingsSectionRequest()

    /// In memory and invented; the real store is never constructed in a demo.
    @MainActor static func demoBook() -> SyncAddressBook {
        func record(
            _ id: String, _ given: String, _ family: String, _ email: String, _ title: String,
            phone: String? = nil, organization: String = ""
        ) -> ExternalRecordFfi {
            ExternalRecordFfi(
                id: id, given: given, family: family, emails: [email],
                phones: phone.map { [$0] } ?? [], organization: organization, title: title)
        }
        return FakeSyncAddressBook(
            access: .authorized,
            records: [
                record("demo-1", "Marisol", "Quint", "marisol@example.com", "Head of finance"),
                record("demo-2", "Ben", "Adeyemi", "ben@example.net", ""),
                record("demo-3", "Zed", "Zimmer", "zed@example.org", "Surveyor", phone: "+1 555 010 0142"),
                record("demo-4", "", "", "office@northwind.example", "", organization: "Northwind Partners"),
            ])
    }

    @ObservationIgnored private(set) var contactsIo: ContactsIoModel?

    @ObservationIgnored private(set) var contactsView: ContactsModel?

    @ObservationIgnored private(set) var contactPopover: ContactPopoverModel?
    @ObservationIgnored private(set) var contactPresenter: ContactPopover?
    @ObservationIgnored private var contactRow: Int?

    private func makeContactPopover(_ session: PostioSession) {
        let model = ContactPopoverModel(engine: session)
        let presenter = ContactPopover(model: model)
        presenter.anchor = { [weak self] in
            guard let self else { return nil }
            if self.contactsView?.isOpen == true, let content = self.mainWindow?.contentView {
                let height = content.bounds.height
                let y = content.isFlipped ? 200 : height - 200
                return (content, NSRect(x: 16 + ContactPopoverView.width / 2, y: y, width: 1, height: 20))
            }
            if self.results?.isOpen == true, let table = self.resultsTable,
                let row = self.contactRow ?? table.cursorTableRow,
                let rect = table.nameAnchor(row: row)
            {
                let centre = max(rect.midX, 16 + ContactPopoverView.width / 2)
                return (table.tableView, NSRect(x: centre, y: rect.minY, width: 1, height: rect.height))
            }
            guard let table = self.focusTable,
                let row = self.contactRow ?? self.focus.cursor,
                let rect = table.nameAnchor(row: row)
            else { return nil }
            // Centred on the name it would hang past the window's left edge (the name starts
            // 56 in, the popover is 400 wide), so it hangs level with the name, 16 in.
            let centre = max(rect.midX, 16 + ContactPopoverView.width / 2)
            return (table.tableView, NSRect(x: centre, y: rect.minY, width: 1, height: rect.height))
        }
        presenter.onToolkitClose = { [weak self, weak session] in
            self?.contactRow = nil
            session?.focusSurfaceClosed(.contactPopover)
        }
        contactPopover = model
        contactPresenter = presenter
    }

    @ObservationIgnored private(set) var threadMenu: ThreadMenuModel?
    @ObservationIgnored private(set) var threadOutline: ThreadOutlineModel?
    @ObservationIgnored private var threadMenuPopover: ThreadMenuPopover?
    @ObservationIgnored private var threadOutlinePopover: ThreadOutlinePopover?

    private func makeThreadPopovers(_ session: PostioSession) {
        let menu = ThreadMenuModel(engine: session)
        threadMenu = menu
        threadMenuPopover = ThreadMenuPopover(model: menu) { [weak session] in
            session?.focusSurfaceClosed(.threadMenu)
        }
        let outline = ThreadOutlineModel(engine: session)
        threadOutline = outline
        threadOutlinePopover = ThreadOutlinePopover(model: outline) { [weak session] in
            session?.focusSurfaceClosed(.outline)
        }
    }

    private func threadButtonAnchor() -> (rect: NSRect, view: NSView)? {
        guard let content = messageContent, content.window != nil else { return nil }
        let frame = verbFrames[ThreadMenuCommand.menu]
            ?? CGRect(x: content.bounds.width - 104, y: 136, width: 88, height: 28)
        let rect = content.isFlipped
            ? frame
            : CGRect(x: frame.minX, y: content.bounds.height - frame.maxY, width: frame.width, height: frame.height)
        return (rect, content)
    }

    private func apply(_ change: ThreadMenuModel.Change) {
        switch change {
        case .open:
            guard let (rect, view) = threadButtonAnchor() else { return }
            threadMenuPopover?.show(relativeTo: rect, of: view)
        case .close:
            threadMenuPopover?.close()
        }
    }

    private func apply(_ change: ThreadOutlineModel.Change) {
        switch change {
        case .open:
            guard let (rect, view) = threadButtonAnchor() else { return }
            threadOutlinePopover?.show(relativeTo: rect, of: view)
        case .redraw:
            threadOutlinePopover?.reload()
        case .focusFilter:
            threadOutlinePopover?.focusFilter()
        case .close:
            threadOutlinePopover?.close()
        }
    }

    private(set) var capture: CaptureModel?

    private static let captureHeight: CGFloat = 500

    private func apply(_ change: CaptureModel.Change) {
        switch change {
        case .open:
            guard let model = capture, let main = mainWindow else { return }
            var measured: CGFloat?
            let hosting = NSHostingView(
                rootView: CaptureView(model: model) { [weak self] height in
                    measured = height
                    guard let self, self.secondary.kind == .capture else { return }
                    self.secondary.fit(contentHeight: height)
                }
                .preferredColorScheme(colorScheme))
            hosting.sizingOptions = []
            // Laid out once before the window exists, so it opens at its content's height.
            hosting.setFrameSize(NSSize(width: CaptureView.width, height: 2000))
            hosting.layoutSubtreeIfNeeded()
            let height = measured.map {
                NSWindow.frameRect(
                    forContentRect: NSRect(x: 0, y: 0, width: CaptureView.width, height: $0),
                    styleMask: [.titled, .closable, .resizable]
                ).height
            } ?? Self.captureHeight
            secondary.show(
                .capture, content: hosting, width: CaptureView.width, height: height,
                title: model.view?.field ?? "", over: main)
        case .redraw:
            break
        case .close:
            secondary.close(.capture)
        }
    }

    @ObservationIgnored private var asking: ConfirmQuestion?

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

    private(set) var picker: PickerModel?
    @ObservationIgnored private var pickerPopover: PickerPopover?
    @ObservationIgnored private weak var messageContent: NSView?
    @ObservationIgnored private var verbFrames: [String: CGRect] = [:]

    private func makePicker(_ session: PostioSession) {
        let picker = PickerModel(engine: session)
        self.picker = picker
        pickerPopover = PickerPopover(model: picker) { [weak session] in
            session?.focusSurfaceClosed(.picker)
        }
    }

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
            // SwiftUI frames are from the content's top left.
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
            if !typedAhead.isEmpty, let editor = field.currentEditor() as? NSTextView {
                let typed = typedAhead
                typedAhead = ""
                editor.insertText(typed, replacementRange: editor.selectedRange())
            }
        case .lines:
            barPanel?.relayout()
            let dropdown = commandBar?.showsDropdown == true ? commandBar?.dropdown : nil
            searchField?.ghost = dropdown?.ghost
            searchField?.ghostHint = dropdown?.ghostHint
            searchField?.operatorTyped = dropdown?.state == .operator
            searchField?.operatorPrefix = dropdown?.operatorPrefix
        case .close:
            barPanel?.hide()
            barShown?(false)
            searchField?.ghost = nil
            searchField?.ghostHint = nil
            searchField?.operatorTyped = false
            searchField?.operatorPrefix = nil
            searchField?.stringValue = ""
            keycapsChanged?()
            typedAhead = ""
        }
    }

    private(set) var searchQuery: SearchQueryModel?

    private(set) var results: ResultsModel?

    @ObservationIgnored private(set) var resultsTable: ResultsTable?

    @ObservationIgnored private(set) var filesGrid: FilesGrid?
    @ObservationIgnored private(set) var filePreview: FilePreview?

    @ObservationIgnored private(set) var filterPopover: FilterPopoverModel?
    @ObservationIgnored private(set) var filterPopoverPresenter: FilterPopover?

    @ObservationIgnored private(set) var quickLook: QuickLookModel?
    @ObservationIgnored private(set) var quickLookPanel: QuickLookPanel?

    private(set) var noResults: NoResultsModel?

    @ObservationIgnored private(set) var savePopover: SavePopoverModel?
    @ObservationIgnored private(set) var savePresenter: SavePopover?

    let searchWords = focusSearchWords()

    @ObservationIgnored var resultsShown: ((Bool) -> Void)?

    @ObservationIgnored weak var queryBox: ResultsQueryBox?

    @ObservationIgnored private var typedAhead = ""

    private func makeResults(_ session: PostioSession) {
        searchQuery = SearchQueryModel(engine: session)
        let results = ResultsModel(engine: session)
        self.results = results
        let resultsTable = ResultsTable(model: results)
        self.resultsTable = resultsTable
        resultsTable.onContactName = { [weak self, weak session] row, address in
            self?.contactRow = row
            session?.focusContactPopoverAt(address)
        }
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

    var resultsChecked: String? {
        guard let selected = results?.selected, selected > 0 else { return nil }
        return focusSearchChecked(n: selected)
    }

    func redrawQuery() {
        guard let query = searchQuery else { return }
        queryBox?.show(chips: query.chips, words: query.words, hint: query.hint, nothingFound: query.ringed)
    }

    func queryFieldFocused() {
        guard commandBar?.isOpen != true else { return }
        run(BarCommand.search)
    }

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
        case .rows, .cursor, .spliced:
            break
        }
    }

    func searchFieldFocused() {
        guard let bar = commandBar, !bar.isOpen else { return }
        run(BarCommand.search)
    }

    func searchFieldTyped(_ text: String) {
        commandBar?.typed(text)
    }

    func searchFieldLeft() {
        guard let bar = commandBar, bar.closedByToolkit() else { return }
        barPanel?.hide()
        barShown?(false)
        searchField?.stringValue = ""
        keycapsChanged?()
        session?.focusSurfaceClosed(.bar)
    }

    func searchFieldCommand(_ selector: Selector) -> Bool {
        guard let bar = commandBar, bar.isOpen else { return false }
        switch selector {
        case #selector(NSResponder.moveUp(_:)):
            bar.move(by: -1)
        case #selector(NSResponder.moveDown(_:)):
            bar.move(by: 1)
        case #selector(NSResponder.insertNewline(_:)):
            if NSApp.currentEvent?.modifierFlags.contains(.command) == true, bar.showAll() {
                return true
            }
            if NSApp.currentEvent?.modifierFlags.contains(.option) == true, bar.excludeHighlighted() {
                return true
            }
            bar.runHighlighted()
        case #selector(NSResponder.insertNewlineIgnoringFieldEditor(_:)):
            if bar.excludeHighlighted() { return true }
            bar.runHighlighted()
        case #selector(NSResponder.insertTab(_:)):
            return bar.tab()
        case #selector(NSResponder.deleteWordBackward(_:)):
            return bar.forgetHighlighted()
        case #selector(NSResponder.cancelOperation(_:)):
            bar.back()
        default:
            return false
        }
        return true
    }

    private func apply(_ change: PlacesModel.Change) {
        switch change {
        case .open:
            placesPopover?.anchor = placesAnchor
            placesPopover?.show()
        case .entries:
            placesPopover?.reload()
        case .place:
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

    let keyMap = KeyMapModel()

    func keyMapDismissed() {
        guard keyMap.closedByToolkit() else { return }
        session?.focusSurfaceClosed(.keyMap)
    }

    private(set) var pendingChord: String?

    private(set) var notice: Notice?

    /// Bumped whenever `notice` is set, so a second identical sentence is a
    /// second notice (*Archived* twice is two things happening).
    private(set) var noticeToken = 0

    func dismissNotice() {
        notice = nil
    }

    private func complain(_ said: String?) {
        guard let said else { return }
        notice = Notice(kind: .refused, message: said, undoable: false)
        noticeToken += 1
    }

    func refreshAccounts() {
        guard let session else { return }
        accounts = session.accounts()
        if accounts.isEmpty, firstRun == nil {
            firstRun = FirstRunModel(session: session)
        } else if !accounts.isEmpty {
            firstRun = nil
        }
        vouch()
    }

    private func vouch() {
        session?.setReachableAccounts(VouchedFor.accounts(accounts, offline: session?.isOffline ?? true))
    }

    /// Created empty if missing: a first run has none and `NSWorkspace` cannot open what does not
    /// exist.
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

    private func consumeEvents(from session: PostioSession) {
        Task { @MainActor [weak self] in
            while let event = await session.nextEvent() {
                self?.handle(event)
            }
        }
    }

    /// The `default:` arm: the event union is append-only, so an older build ignores what it does
    /// not know (ADR 0019 Q7).
    private func handle(_ event: UiEvent) {
        guard case .open = state else { return }
        // Before the switch: completions, undos and refusals are the controller's toast; only a
        // failure is a notice.
        if let arriving = Notice(event), arriving.shownBesideFocusToast {
            notice = Notice.winner(showing: notice, arriving: arriving)
            noticeToken += 1
        }
        if let change = commandBar?.apply(event) {
            apply(change)
            return
        }
        if let change = places?.apply(event) {
            apply(change)
            return
        }
        // Before the surfaces: the controller put it on its stack, so its opening is not reported
        // back.
        if keyMap.apply(event) != nil { return }
        if let change = picker?.apply(event) {
            apply(change)
            return
        }
        if filtered?.apply(event) != nil { return }
        if let change = digest?.apply(event) {
            apply(change)
            return
        }
        if let change = ruleSheet?.apply(event) {
            apply(change)
            return
        }
        if let change = contactPopover?.apply(event) {
            if change == .close { contactRow = nil }
            contactPresenter?.apply(change)
            return
        }
        if contactsSettings?.apply(event) == true { return }
        if contactsView?.apply(event) != nil { return }
        if case let .focusWriteTo(address) = event {
            if let encoded = address.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed),
                let url = URL(string: "mailto:\(encoded)"), let mailto = Mailto(url)
            {
                _ = write(mailto: mailto)
            }
            return
        }
        if let change = capture?.apply(event) {
            apply(change)
            return
        }
        if let change = composer?.apply(event) {
            apply(change)
            return
        }
        if case let .focusQuery(view) = event {
            messageWindow?.searchWords = view.words
            searchQuery?.apply(event)
            redrawQuery()
            return
        }
        if let change = filterPopover?.apply(event) {
            filterPopoverPresenter?.apply(change)
            return
        }
        if let change = savePopover?.apply(event) {
            savePresenter?.apply(change)
            return
        }
        if let change = quickLook?.apply(event) {
            quickLookPanel?.apply(change, over: mainWindow)
            return
        }
        // Leaving the results is also the results' event, so it is told and handed on.
        if case .focusRelaxations = event {
            noResults?.apply(event)
            return
        }
        if case .focusLeaveResults = event { noResults?.apply(event) }
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
        if let change = focus.apply(event) {
            focusTable?.apply(change)
            // Every verb and every undo says a toast, and either may have
            // moved the stack's top.
            if change == .toast { refreshUndo() }
            return
        }
        if let change = states.apply(event) {
            // An empty inbox has no first page to land: the list under the
            // page takes the keyboard when the page is drawn.
            if change == .empty, states.empty != nil {
                DispatchQueue.main.async { [weak self] in self?.listTakesTheKeyboard() }
            }
            return
        }
        if let change = threadMenu?.apply(event) {
            apply(change)
            return
        }
        if let change = threadOutline?.apply(event) {
            apply(change)
            return
        }
        if let surface = FocusIntents.surface(event) {
            apply(surface)
            return
        }
        switch event {
        case let .focusListChanged(total):
            focusTable?.listChanged(total: total)
            refreshCounts()
        case let .focusSplice(anchor, children):
            focusTable?.splice(anchor: Int(anchor), children: children.count)
        case .focusUnsplice:
            focusTable?.unsplice()
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
            if menuPlan.apply(event) { mountMenuBar() }
            if keyMap.isOpen, let session { keyMap.refresh(session.focusKeyMap()) }
        case .noticesChanged:
            notifications.unread(focusStrip?.unread, badge: session?.notices().badge ?? true)
        case .surfacedChanged:
            refreshCounts()
        case let .newMail(account, mailbox, messages):
            mailMoved()
            arrived(MailArrival(account: account, mailbox: mailbox, messages: messages))
        case .messagesChanged, .messagesRemoved, .messageListChanged, .mailboxesChanged:
            mailMoved()
        case let .bodyLoaded(_, message), let .bodyFailed(_, message),
             let .bodyReachable(_, message):
            messageWindow?.bodyLanded(message)
        case let .reindexProgress(_, done, total):
            settingsActions.reindexProgressed(done: done, total: total)
        case let .focusConfirm(confirm):
            ask(ConfirmQuestion(confirm))
        case let .focusRun(command):
            run(command)
        default:
            break
        }
    }

    @ObservationIgnored
    private var listHadKeyboard = false

    /// Once, on the list's first page or an empty inbox: the table takes the keyboard. Left to
    /// AppKit, the first key view (Inbox ▾, which Space would press) had it.
    private func listTakesTheKeyboard() {
        guard !listHadKeyboard, let table = focusTable?.tableView, let window = table.window else { return }
        listHadKeyboard = true
        guard commandBar?.isOpen != true, picker?.isOpen != true,
              !(window.firstResponder is NSTextView)
        else { return }
        window.makeFirstResponder(table)
    }

    @ObservationIgnored
    private var demoKeys = DemoMode.keys

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

    private func drawDemoSnapshot() {
        guard let path = DemoMode.snapshot else { return }
        Task { @MainActor [weak self] in
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            guard let window = self?.mainWindow, let frame = window.contentView?.superview else { return }
            frame.layoutSubtreeIfNeeded()
            guard let rep = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) else { return }
            frame.cacheDisplay(in: frame.bounds, to: rep)
            // The bar's panel and popovers are windows of their own, so they are drawn in at their
            // place.
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

    /// A demo's click on a filter button (`@from`, `@to`, `@date`, `@anywhere`, `@label`,
    /// `@reply`):
    /// the `POSTIO_DEMO_KEYS` format has no pointer. `false` for any other key.
    private func replayClick(_ key: KeyEvent.Reduced) -> Bool {
        guard key.name == nil, let word = key.character, word.hasPrefix("@"), word.count > 1 else { return false }
        let kinds: [String: FilterKindFfi] = [
            "@from": .from, "@to": .to, "@date": .date, "@anywhere": .anywhere, "@label": .label,
        ]
        if word.lowercased() == "@reply" {
            _ = session?.demoArrive(what: "reply")
            return true
        }
        guard let kind = kinds[word.lowercased()] else { return false }
        searchQuery?.tap(kind)
        return true
    }

    private func replayIntoField(_ key: KeyEvent.Reduced) -> Bool {
        let plain = !key.modifiers.command && !key.modifiers.control && !key.modifiers.option
        guard plain else { return false }
        if let picker, picker.isOpen, let popover = pickerPopover {
            switch key.name {
            case "down": picker.move(by: 1)
            case "up": picker.move(by: -1)
            case nil:
                let text = key.character ?? ""
                let picks = text == " " || (text.count == 1 && text.allSatisfy(\.isNumber))
                let typing = picker.inField || (picker.filters && !(picker.typed.isEmpty && picks))
                guard typing else { return false }
                popover.type(text)
            default: return false
            }
            return true
        }
        if let menu = threadMenu, menu.isOpen {
            switch key.name {
            case "down": menu.move(by: 1)
            case "up": menu.move(by: -1)
            case "return": menu.confirm()
            default: return false
            }
            return true
        }
        if let outline = threadOutline, outline.isOpen, let popover = threadOutlinePopover {
            switch key.name {
            case "down": outline.move(by: 1)
            case "up": outline.move(by: -1)
            case "return": outline.show()
            case nil: popover.type(key.character ?? "")
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

    private func mailMoved() {
        mailboxes = session?.mailboxes ?? []
        refreshCounts()
    }

    private func arrived(_ arrival: MailArrival) {
        let notices = session?.notices()
        let decision = MailNotifier.decide(
            arrival,
            showing: nil,
            isActive: NSApplication.shared.isActive,
            mailboxName: mailboxes.first { $0.id == arrival.mailbox }?.name,
            enabled: notices?.notify ?? true,
            role: mailboxes.first { $0.id == arrival.mailbox }?.role ?? .regular,
            notifyRoles: notices?.notifyRoles ?? ["inbox"]
        )
        guard case let .deliver(notification) = decision else { return }
        notifications.post(notification)
    }

    @ObservationIgnored
    private lazy var menuPlan = MenuBarPlan { [weak self] command in
        self?.session?.bindings(for: command) ?? []
    }

    /// Accelerators come from the session's keymap and are absent before one exists; dispatch is
    /// the key monitor's.
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
                    return Intercepted.all.contains(id)
                }
                return session.isAvailable(id, in: self.context)
            },
            run: { [weak self] id in self?.run(id) },
            undo: undoRouter
        )
    }

    private func installKeyboard(_ session: PostioSession) {
        let monitor = KeyMonitor(
            resolve: { [weak self] reduced, context, typing in
                guard let self, let session = self.session else { return .unhandled }
                let shown = self.keyWindow.current == .compose
                    && self.composer?.model?.chooserHasKeyboard == true
                var outcome = KeyOutcomeFfi.unhandled
                for asked in ForwardChooserKeys.contexts(base: context, chooserFocused: shown, typing: typing) {
                    outcome = session.key(reduced, in: asked, typing: typing)
                    if case .unhandled = outcome { continue }
                    break
                }
                return outcome
            },
            run: { [weak self] id in self?.run(id) ?? false },
            pending: { [weak self] description in self?.pendingChord = description },
            context: { [weak self] in self?.context ?? .list }
        )
        monitor.start()
        keys = monitor
    }

    @discardableResult
    func run(_ id: String, on target: Int64? = nil) -> Bool {
        // The banner's password sheet has the keyboard, not the list behind it.
        if bannerRepair.asking != nil { return false }
        if let io = contactsIo, io.isOpen {
            switch id {
            case Intercepted.back: io.done()
            case "contacts_suggestions": io.review()
            default: return true
            }
            return true
        }
        if asking != nil { return false }
        if let save = savePopover, save.isOpen {
            switch id {
            case Intercepted.back: save.cancel()
            case "open_message", "picker_confirm": save.save()
            default: return false
            }
            return true
        }
        if let sheet = ruleSheet, sheet.isOpen {
            switch id {
            case Intercepted.back: session?.invoke(id)
            case "open_message", "picker_confirm": sheet.create()
            default: return false
            }
            return true
        }
        // Space and Return in a picker are about the highlighted row, which only the popover knows.
        if let picker, picker.isOpen {
            if picker.run(id) { return true }
            if id == Intercepted.back {
                picker.back()
                return true
            }
        }
        switch id {
        case Intercepted.back where keyWindow.current == .compose && composer?.model?.suggesting != nil:
            composer?.model?.dismissSuggestions()
        case Intercepted.back where keyWindow.current == .message && messageWindow?.showingSource == true:
            // Esc from the raw source returns to the message; the controller does not know the
            // source is up.
            messageWindow?.closeSource()
        case Intercepted.unsubscribe where keyWindow.current == .message && messageWindow?.displayed != nil:
            // The one deliberate act, recorded by the boundary (#1585); the controller has no
            // message window to do it in.
            messageWindow?.unsubscribe()
        case Intercepted.openAttachmentOrLink where keyWindow.current == .message && messageWindow?.displayed != nil:
            // A menu at the window's centre; nothing opens until one is chosen.
            messageWindow?.offerAttachments { [weak self] choices in
                self?.presentAttachmentMenu(choices)
            }
        case Intercepted.clearListMail where keyWindow.current == .message && messageWindow?.displayed != nil:
            messageWindow?.takeUpOffer()
        case Intercepted.back where placesPopover?.isShown == true:
            placesPopover?.close()
        case Intercepted.contactsSync:
            settingsSection.ask("contacts")
            settingsWindow.raise()
        case Intercepted.contactsImport:
            contactsIo?.runImport()
        case Intercepted.contactsExport:
            contactsIo?.runExport()
        case Intercepted.settings:
            // Only a view can open a window: the main window turns this request into
            // `openWindow(id:)` (#1261).
            settingsWindow.raise()
        // Aim at the row the settings keyboard is on. A missing cursor is a real answer: falling
        // back to the first account would remove somebody's mail on a keystroke aimed at nothing.
        case Intercepted.addAccount:
            settingsWindow.raise()
            settingsAccounts.ask(.add)
        case Intercepted.editConfig:
            openConfigFile()
        case Intercepted.updateCredential where keyWindow.current == .main && states.banner?.account != nil:
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
            if keyWindow.current == .compose,
               let composer = composer?.model,
               ComposeCommands.run(id, on: composer, through: session)
            {
                if composer.sent { secondary.close(.composer) }
                return true
            }
            // Set by the person: the read clock stops, so a message kept unread stays unread while
            // open.
            if id == "toggle_read" { messageWindow?.readStateSet() }
            session?.invoke(id)
        }
        return true
    }

    /// Held until the list has landed: a cold launch from a captured line has no session to ask.
    @ObservationIgnored
    private var waitingLinks = PostioLink.Waiting()

    @ObservationIgnored
    private var listLanded = false

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

    /// `false` when there is nothing to open one from.
    func write(mailto: Mailto) -> Bool {
        guard let session, let composer, let draft = session.mailtoDraft(mailto) else { return false }
        composer.open(own: draft)
        showComposerWindow()
        return true
    }

    /// Stops the engines, then drops the store: dropping an engine at process exit lets libcrypto
    /// go away under a thread still encrypting a page (SQLCipher).
    func shutdown() {
        keys?.stop()
        keys = nil
        reachability.stop()
        session?.shutdown()
        session = nil
    }
}
