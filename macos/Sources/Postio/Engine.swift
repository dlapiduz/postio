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

    /// The `[ui]` table this session was opened with.
    ///
    /// Observed, because the theme is drawn from it and a change has to
    /// repaint. `nil` until a session opens, which is the window's honest
    /// state before then: it follows the system.
    private(set) var appearance: AppearanceFfi?

    /// The configured accounts, for the settings window's Accounts pane.
    ///
    /// Read once when the session opens. Nothing in this build changes them
    /// -- adding an account is still `postio-provision` (#649) -- so there is
    /// nothing yet to keep this in step with.
    private(set) var accounts: [AccountFfi] = []

    /// The colour scheme `[ui].theme` asks for, or `nil` to follow the system.
    ///
    /// `system` and "no session yet" are the same answer on purpose — both
    /// mean "Postio has no opinion", and SwiftUI spells that `nil`.
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
        // Before the store, deliberately. Opening it waits on the Keychain and
        // can wait forever — and while it does, the application is on screen
        // with a menu bar. Installed after the session, that bar was AppKit's
        // stock one for the whole of the wait and for the entire life of a
        // build whose store never opened (#1262). The registry is a `const`
        // table and needs no session to read.
        installMenuBar()
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
            let controller = MessageTableController(source: SessionRowSource(session: session))
            // What `[ui]` says, applied before the first row is drawn. The
            // settings pane writes this table; if nothing read it here, a Mac
            // user would pick Compact and watch the list not change (#1215).
            let appearance = session.appearance()
            controller.ui = appearance
            // From this session's keymap, so a rebinding reaches the row.
            controller.hints = session.rowHints()
            accounts = session.accounts()
            vouch()
            reloadSavedSearches()
            self.appearance = appearance
            state = .open(controller)
            // Nothing was ever fetched before this: the store opened and
            // stayed empty because no engine had been started (#648).
            mailboxes = session.mailboxes
            if let started = try? session.startSyncing(), started > 0 {
                // Engines run on their own runtime; the list repaints from
                // events rather than from anything awaited here.
            }
            // An account added while this is running writes its row and gets
            // no engine, because the engines were started just above. Saying
            // so here is what makes it sync without a relaunch (#1299).
            settingsActions.accountAdded = { [weak self] in
                guard let self, let session = self.session else { return }
                self.accounts = session.accounts()
                self.vouch()
                _ = try? session.startSyncing()
                self.mailboxes = session.mailboxes
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
            // Resting on a message marks it read; sweeping past marks
            // nothing. The delay and the arming rule are `postio_ui::dwell`'s
            // — see `DwellClock`, which owns only the timer.
            dwell = DwellClock { [weak self] message in
                self?.session?.markReadOnDwell(message)
            }
            // Again, now that there are bindings to draw: the bar installed
            // at launch shows the built-in defaults, and a rebound key has to
            // reach the menu.
            installMenuBar()
            // The platform observes and the engine is told. Callbacks arrive
            // on a background queue and may repeat the same answer; the
            // boundary absorbs that, nudging a reconnect only on a real
            // transition back, so there is nothing to debounce here.
            reachability.start { [weak self] offline in
                Task { @MainActor in
                    self?.session?.setOffline(offline)
                    // The same signal, said the other way: `⌘A` in the
                    // unified list is scoped to what Postio can vouch for,
                    // and until this was reported the boundary's safe
                    // default meant it selected nothing at all (#811).
                    self?.vouch()
                }
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

    /// Asked for the settings window. Watched by the shell, which is what
    /// can actually open one.
    private(set) var settingsWindow = WindowRequest(id: WindowId.settings)

    /// The message a notification click asked for, for the shell to open.
    ///
    /// A tuple rather than a struct because nothing else reads it, and paired
    /// with a counter because two clicks on the same notification are two
    /// requests: SwiftUI's `onChange` compares values, and the second would
    /// otherwise look like nothing happened.
    private(set) var requested: (mailbox: Int64, message: Int64?)?
    private(set) var requestedToken = 0

    /// The messages being written, and the windows they are waiting for.
    let compose = ComposeStore()

    /// What the settings window's account actions are doing, held here
    /// because their progress arrives as events and a window that owned them
    /// would have to be open at the moment one landed.
    let settingsActions = AccountActions()

    /// Which account row the settings window's keyboard is on, and the two
    /// sheets a command can ask that window for. See `SettingsAccounts`.
    let settingsAccounts = SettingsAccounts()

    /// The saved searches in the sidebar, and which one the keyboard is on.
    /// See `SavedSearches`.
    let savedSearches = SavedSearches()

    /// Measured body heights, session-lived, so a revisited message opens
    /// at full size instead of popping from the minimum.
    let bodyHeights = BodyHeights()

    /// Putting a broken account back in service. Held here because
    /// `update_credential` is a command, and a command cannot reach a view.
    let accountRepair = AccountRepair()

    /// The conversation the reading pane is showing (#1263).
    ///
    /// Held by the engine rather than by the view so that an event can fill
    /// it: the read is asynchronous, and a pane that owned the model would
    /// have to be on screen at the moment the answer arrived.
    let conversation = ConversationModel()

    private let notifications = MailNotifications()
    private let reachability = Reachability()
    private var keys: KeyMonitor?
    /// The clock that decides a message has been read (#71, #1159).
    private var dwell: DwellClock?

    /// Which pane has the keyboard.
    ///
    /// Reported by the views as they take focus rather than inferred from the
    /// responder chain: `Pane` is Postio's vocabulary of surfaces and AppKit
    /// knows nothing about it, so a mapping from view classes would be this
    /// application guessing at its own state. The list is where the keyboard
    /// starts.
    private(set) var pane: Pane = .list

    /// Which surface the resolver should answer for **inside the main
    /// window**.
    ///
    /// Follows the focused pane, except while an overlay is up — a key
    /// pressed in the palette must not resolve as the list, or typing a
    /// command's name would archive mail.
    var paneContext: UiContext = .list

    /// Which window has the keyboard.
    ///
    /// The key monitor is a *local* monitor: it sees every key press in the
    /// application, compose windows included. Until this existed it asked
    /// only for the pane, so `UiContext.composer` was never the answer —
    /// every composer binding resolved to nothing and the list's own verbs
    /// kept resolving while a message was being written.
    let keyWindow = KeyWindowTracker()

    /// Which surface the resolver should answer for.
    ///
    /// The window decides first; the pane only gets a say when the keyboard
    /// is in the window that has panes. See `KeyboardContext`.
    var context: UiContext {
        KeyboardContext.resolving(keyWindow: keyWindow.current, mainWindow: paneContext)
    }

    /// Move the keyboard to `pane`.
    func focus(_ pane: Pane) {
        self.pane = pane
        paneContext = contextOf(pane)
    }

    /// What `pane` resolves keys as.
    ///
    /// The list is two contexts, not one: over a result set it is
    /// `Context::Search`, which is where `o` and `⌘⇧S` live. See
    /// `SearchContext` for why that cannot be read off the query field's
    /// focus.
    private func contextOf(_ pane: Pane) -> UiContext {
        guard pane == .list else { return pane.context }
        return SearchContext.list(showingResults: session?.isSearching ?? false)
    }

    /// Re-read the list's context after a search ran or was cleared.
    ///
    /// Running a search does not move the keyboard, so nothing else would
    /// notice that the list is now a result set.
    func searchChanged() {
        // Before the guard: the stamp is about the *result set* changing,
        // which is true whether or not the field holds the keyboard. The
        // refine bar and the field's readout both follow it.
        searchStamp += 1
        searchFacets.resultsChanged(searching: session?.isSearching == true)
        // A saved search holds the keyboard only while its results are up.
        // Once the list is a folder again the highlight goes back to the
        // folder, rather than staying on a query nothing is showing.
        if session?.isSearching != true { savedSearches.put(cursor: nil) }
        guard !showingSearch, !showingParts else { return }
        paneContext = contextOf(pane)
    }

    /// How many searches have run, however they ran — typed, refined,
    /// re-ordered, or picked from the sidebar. What the refine bar
    /// re-measures on, and what makes the toolbar field adopt a query it
    /// did not run itself.
    private(set) var searchStamp = 0

    /// The scope rail's rows and the refine chips — see `SearchFacets`.
    let searchFacets = SearchFacets()

    /// Whether the list is a result set rather than a folder.
    ///
    /// Reads the stamp so SwiftUI asks again after a search runs or clears:
    /// `isSearching` is the boundary's answer, and a computed property over
    /// it is not something SwiftUI can observe on its own.
    var showingResults: Bool {
        _ = searchStamp
        return session?.isSearching ?? false
    }

    /// Which scope the search is looking in, for the rail to mark.
    var searchScope: SearchScopeFfi {
        _ = searchStamp
        return session?.searchScope ?? .allMail
    }

    /// Look in `scope` — a row of the rail. The same query, asked again.
    func setSearchScope(_ scope: SearchScopeFfi) {
        guard let session, session.isSearching else { return }
        session.setSearchScope(scope)
        listChanged()
        searchChanged()
    }

    /// Walk the rail from the keyboard: the sidebar is showing it, not the
    /// folders. See `SearchFacets.step`.
    private func stepScope(by delta: Int) -> Bool {
        guard let next = searchFacets.step(from: searchScope, by: delta) else { return false }
        if next != searchScope { setSearchScope(next) }
        return true
    }

    /// Measure the rail's counts and the chips for the results on screen.
    func measureFacets() async {
        guard let session, session.isSearching else { return }
        let measured = await Task.detached { session.searchFacets() }.value
        // A search cleared while this was measuring has nothing to draw it on.
        guard !Task.isCancelled, session.isSearching else { return }
        searchFacets.take(measured)
    }

    /// Narrow the current search by one token — a refine chip.
    ///
    /// Through the session's own query, not the field's text: the field is
    /// display here, and the query that ran is the boundary's answer. The
    /// stamp then makes the field adopt the result, so editing it afterwards
    /// starts from the refined query rather than silently dropping the
    /// narrowing.
    func refineSearch(_ token: String) {
        guard let session, let query = session.searchQuery else { return }
        session.search(query.isEmpty ? token : "\(query) \(token)")
        listChanged()
        searchChanged()
    }

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

    /// Whether the search field has the keyboard.
    ///
    /// A surface, like the palette, and handled here for the same reason: a
    /// session cannot present one. What it is *over* is the boundary's, which
    /// is why this is the only search state Swift keeps.
    ///
    /// **It moves the key context with it**, and that is the point of the
    /// property. `/` set the context and a *click* into the field did not, so
    /// the two ways into search left the application in two different states:
    /// with the field click-focused, `Save search as folder` and `Toggle
    /// result order` were drawn disabled (their registry contexts are
    /// `Context::Search`), and the `Escape` arm below could not match. GTK
    /// says the same thing from the other side — *"focusing the field **is**
    /// opening the box: a user who clicks it has asked the same question `/`
    /// asks"*.
    /// How many times search has been asked for.
    ///
    /// A count beside the Bool, because the field is always on the toolbar:
    /// `showingSearch` can already be true when `/` is pressed again, and a
    /// value that does not change cannot carry the ask — the wish-token
    /// lesson, applied to focus.
    private(set) var searchFocusAsks = 0

    var showingSearch = false {
        didSet {
            guard showingSearch != oldValue else { return }
            paneContext = showingSearch ? .search : contextOf(pane)
        }
    }

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

    /// What Postio last said back about something you asked it to do.
    ///
    /// `nil` when there is nothing to say. The four outcome events — an
    /// action ran, an undo was applied, a verb was refused, something failed
    /// — all arrived here as `Other` and were dropped, so the application did
    /// the work and never answered.
    private(set) var notice: Notice?

    /// Bumped whenever `notice` is set, so a second identical sentence is a
    /// second notice.
    ///
    /// *Archived* twice in a row is two things happening, and a view watching
    /// the value alone would see nothing the second time — the same lesson
    /// `WindowRequest` records about `onChange`.
    private(set) var noticeToken = 0

    /// Take the notice down.
    ///
    /// Called by whatever is drawing it once its time is up, and by the Undo
    /// button on its way out.
    func dismissNotice() {
        notice = nil
    }

    /// Which folders are collapsed in the sidebar.
    ///
    /// Held here rather than left inside SwiftUI's `DisclosureGroup`, for two
    /// reasons that are really one: the keyboard walk must not step onto a
    /// row nobody can see, and `toggle_folder` needs something to toggle.
    /// State a command has to reach cannot live inside a view.
    private(set) var collapsedFolders: Set<SidebarRowId> = []

    /// Open or close `row`'s children.
    func toggleCollapsed(_ row: SidebarRowId) {
        if collapsedFolders.contains(row) {
            collapsedFolders.remove(row)
        } else {
            collapsedFolders.insert(row)
        }
    }

    func setCollapsed(_ row: SidebarRowId, _ collapsed: Bool) {
        if collapsed { collapsedFolders.insert(row) } else { collapsedFolders.remove(row) }
    }

    /// Every sidebar row, in the order it is drawn — saved searches included.
    var sidebarOrder: [SidebarWalk.Stop] {
        SidebarWalk.visible(
            special: specialFolders,
            saved: savedSearches.rows,
            roots: folderRoots,
            children: { [weak self] parent in self?.children(of: parent) ?? [] },
            collapsed: collapsedFolders
        )
    }

    /// The folder half of where the sidebar's keyboard is.
    ///
    /// Separate from the folder in view: stepping past a `\Noselect`
    /// container moves the keyboard onto it without opening anything, so the
    /// two answers differ for exactly as long as it takes to press `j` again.
    ///
    /// **And the sidebar's highlight is drawn from it**, through
    /// `highlightedFolder`. It used to be a `@State` in the shell that only a
    /// click could move, so `j`, `k` and the four `g` destinations opened a
    /// folder while the highlight stayed on the last one clicked — and a click
    /// never reached this, so the next `j` stepped on from wherever the
    /// keyboard had last been rather than from the row under the pointer.
    private(set) var folderCursor: SidebarRowId?

    /// Where the sidebar's keyboard is: a folder or a saved search.
    var sidebarCursor: SidebarCursor? {
        SidebarWalk.cursor(folder: folderCursor, savedSearch: savedSearches.cursor)
    }

    /// The folder the sidebar draws as selected, if any.
    var highlightedFolder: SidebarRowId? {
        SidebarWalk.highlightedFolder(folder: folderCursor, savedSearch: savedSearches.cursor)
    }

    /// A folder row was picked — by a click, a restore, or a notification.
    ///
    /// The one way in for everything that is not the keyboard's walk, so the
    /// highlight, the cursor and the list cannot come to disagree.
    func pick(_ row: SidebarRowId?) {
        // Picking the row that is already picked changes nothing, as it did
        // when this was a selection `onChange` watched: reopening would throw
        // away the list's place for a click that asked for nothing new.
        guard let row, row != highlightedFolder,
              let folder = mailboxes.first(where: { $0.rowId == row })
        else { return }
        land(on: .folder(folder))
    }

    /// A saved search's row was picked.
    func pick(_ search: SavedSearchFfi) {
        land(on: .savedSearch(search))
    }

    /// Put the sidebar's keyboard on `stop`, and do what landing there does.
    private func land(on stop: SidebarWalk.Stop) {
        switch stop {
        case let .folder(folder):
            savedSearches.put(cursor: nil)
            folderCursor = folder.rowId
            if SidebarWalk.opens(folder) { open(folder) }
        case let .savedSearch(search):
            // The folder half stays where it was, so leaving the search puts
            // the highlight back on the folder whose mail the list returns to.
            savedSearches.put(cursor: search.key)
            open(search)
        }
    }

    /// Move the sidebar's keyboard by `delta`, opening what it lands on.
    private func stepSidebar(by delta: Int) -> Bool {
        guard let landed = SidebarWalk.step(from: sidebarCursor, in: sidebarOrder, by: delta) else {
            return false
        }
        land(on: landed)
        return true
    }

    /// What the reader held back for `message`, as counts.
    ///
    /// Zeroes when nothing was, which is what makes "Render once" not
    /// appear: only the markup part can load anything, and offering to
    /// render an `image/png` once would be theatre.
    func heldBack(for message: Int64) -> (remote: UInt32, trackers: UInt32) {
        // A full blocked render, once, when the panel opens — a
        // user-initiated surface, not the j/k hot path. The notice rides
        // the document's answer now (#1589), and the panel is the one
        // caller with no render of its own to take it from.
        guard let notice = session?.readerDocument(message: message, remote: .blocked, original: true).notice
        else { return (0, 0) }
        return (notice.remoteImages, notice.trackers)
    }

    /// Run a saved search — picking its row in the sidebar.
    ///
    /// The same call the query field makes. A saved search is a query that
    /// was written down, not a second kind of thing to open, and a separate
    /// path here would be a second answer to what a query means.
    func open(_ search: SavedSearchFfi) {
        guard let session else { return }
        session.search(search.query)
        listChanged()
        searchChanged()
    }

    /// Re-read `config.toml`'s saved searches.
    ///
    /// The file is hand-edited and watched, so this reads it rather than
    /// trusting a copy: patching something read when the window opened would
    /// write an hour-old `[sync]` block back over a newer one.
    func reloadSavedSearches() {
        guard let path = try? settingsPath() else { return }
        savedSearches.load(from: path)
    }

    /// Run one of the saved-search verbs against the file, and take what it
    /// answers.
    private func editSavedSearch(_ work: (String) throws -> SavedSearchEditFfi) {
        guard let path = try? settingsPath() else {
            complain("Postio could not work out where its configuration file lives.")
            return
        }
        do {
            savedSearches.apply(try work(path))
        } catch let error as SettingsError {
            // One variant, because there is one thing to do about any of
            // them: show what the parser or the filesystem said and leave
            // what is on screen alone.
            let said = switch error {
            case let .Invalid(message): message
            }
            complain(said)
        } catch {
            complain("\(error)")
        }
    }

    /// Call a saved search something else.
    func renameSavedSearch(_ key: String, to name: String) {
        editSavedSearch { try PostioFFI.renameSavedSearch(path: $0, key: key, name: name) }
    }

    /// Take a saved search out of the file, having asked.
    func deleteSavedSearch(_ key: String) {
        editSavedSearch { try PostioFFI.deleteSavedSearch(path: $0, key: key) }
        // The row is gone; the keyboard must not still claim to be on it.
        if savedSearches.focused == nil { savedSearches.put(cursor: nil) }
    }

    /// Say what went wrong, if anything did.
    ///
    /// The settings verbs answer with a sentence or with nothing, and the
    /// sentence is the boundary's. A refusal that vanished would leave a
    /// switch that looks broken.
    private func complain(_ said: String?) {
        guard let said else { return }
        notice = Notice(kind: .refused, message: said, undoable: false)
        noticeToken += 1
    }

    /// Re-read the accounts after one of them changed.
    ///
    /// The rows are the boundary's answer, not a local copy to patch: a pane
    /// that edited its own array would be drawing what it believes rather
    /// than what was written.
    func refreshAccounts() {
        guard let session else { return }
        accounts = session.accounts()
        // Switching an account off takes it out of what the unified view
        // can vouch for, and the switch is right here.
        vouch()
    }

    /// Tell the boundary which accounts the unified view can vouch for.
    ///
    /// Called from the two things that change the answer: the connection
    /// moving, and the accounts list changing. See `VouchedFor`.
    private func vouch() {
        session?.setReachableAccounts(VouchedFor.accounts(accounts, offline: session?.isOffline ?? true))
    }

    /// Open `config.toml` in whatever edits it.
    ///
    /// The path is `postio-config`'s, per platform and per
    /// `$XDG_CONFIG_HOME` — a frontend that guessed would open a file
    /// nothing loads. Created empty if it is not there yet, because a first
    /// run has none and `NSWorkspace` cannot open what does not exist.
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

    /// Go to the folder for `role`, or say there is none.
    private func goTo(_ role: MailboxRoleFfi) -> Bool {
        guard let found = SidebarWalk.destination(role, among: sidebarOrder) else {
            // Said rather than swallowed. GTK announces the same thing, for
            // the same reason: a key that silently does nothing cannot be
            // told from one that is broken.
            notice = Notice(
                kind: .refused,
                message: "This account has no \(mailboxRoleName(role: role)) folder.",
                undoable: false
            )
            noticeToken += 1
            return true
        }
        land(on: .folder(found))
        return true
    }

    /// The special-use folders, in the order the boundary put them in.
    ///
    /// Inbox first, then the canvas' order — and one row per role, however
    /// many folders carry it. Both decisions are `postio_ui::sidebar`'s and
    /// neither is re-made here: sorting in Swift would be a second answer to
    /// "where is my inbox", and the duplicate rule took a bug report to find
    /// on the other frontend (#501, #1155).
    var specialFolders: [MailboxFfi] {
        mailboxes.filter(\.special)
    }

    /// Ordinary folders with no parent, as a tree's roots.
    ///
    /// A folder whose role is already represented above appears here under
    /// its server name rather than being dropped — it is still real mail and
    /// still reachable.
    var folderRoots: [MailboxFfi] {
        mailboxes.filter { !$0.special && $0.parent == nil }
    }

    /// The accounts that have ordinary folders to show, in the order the
    /// boundary listed them.
    var accountsWithFolders: [AccountFfi] {
        let present = Set(folderRoots.map(\.account))
        return accounts.filter { present.contains($0.id) }
    }

    /// Whether a sync pass is running now.
    ///
    /// From `SyncProgress`, which arrives while a pass is in flight and stops
    /// when it is done — the presence of progress is the answer to "is
    /// anything happening", which is the trap `postio-gtk`'s footer fell into
    /// by reading a `last_synced_at` that only moves when a pass *completes*.
    private(set) var syncing = false

    /// Why this account cannot sign in, or `nil` while it can.
    ///
    /// Not the same as [`isOffline`](Self.isOffline), which is the
    /// *machine's* reachability from `NWPathMonitor`. This is the account's,
    /// it is the one that needs a person, and it had nowhere to live: the
    /// `ConnectionChanged` payload was dropped on the floor.
    private(set) var failure: FailureReasonFfi?

    /// Whether the platform has told the engine there is no connection.
    var isOffline: Bool { session?.isOffline ?? false }

    /// The children of `parent`, ordinary folders only.
    func children(of parent: Int64) -> [MailboxFfi] {
        mailboxes.filter { !$0.special && $0.parent == parent }
    }

    /// Show a folder's messages.
    ///
    /// Re-scoping drops the selection on the other side, which is right:
    /// "these twelve" means something else the moment the list does, and an
    /// action carrying a selection across would land on mail the user cannot
    /// see.
    func open(_ row: MailboxFfi) {
        guard let session, case let .open(controller) = state else { return }
        // A view row is a query, not a folder — `showingMailbox` is what
        // decides whether new mail is already on screen, and "Flagged" is
        // not an answer to "which folder did this arrive in".
        showingMailbox = row.isView ? nil : row.id
        session.openScope(SidebarScope.of(row))
        listVersion += 1
        controller.tableView?.reloadData()
    }

    /// Show a folder by id, for a caller that has one and not a row.
    func open(mailbox: Int64) {
        guard let row = mailboxes.first(where: { $0.id == mailbox && !$0.isView }) else { return }
        open(row)
    }

    /// Bumped whenever the open list's contents change.
    ///
    /// **SwiftUI needs something it can observe, and a row count is not it.**
    /// `rowCount` reads through to the boundary, so it is a computed property
    /// over a `session` reference that never changes — which means the shell's
    /// `if engine.rowCount == 0` was evaluated once, when the folder opened
    /// empty, and never again. `reloadData()` refreshed the *table* inside a
    /// branch SwiftUI had already decided not to draw: 37 unread in the
    /// sidebar, "No messages" beside it (#1150).
    ///
    /// A counter rather than a cached count, because the count itself belongs
    /// to the boundary and a second copy here is a second thing to be wrong.
    /// This says only *that* it changed; `rowCount` still says what it is.
    private(set) var listVersion = 0

    /// How many rows the open scope has, or zero when there is no session.
    ///
    /// Reads `listVersion` so that SwiftUI registers a dependency on it: this
    /// is a computed property, and what a view actually observes is whatever
    /// stored property it touches on the way through.
    var rowCount: UInt32 {
        _ = listVersion
        return session?.rowCount ?? 0
    }

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
        // Before the switch, because it is true of several arms and was
        // previously true of exactly one: the sidebar's counts move with read
        // state and with mail arriving or leaving, not only when the folder
        // set changes. `SidebarCounts` is the rule, and it has a test;
        // putting it in an arm below would put it back where nothing can
        // reach it.
        if SidebarCounts.movedBy(event) {
            mailboxes = session?.mailboxes ?? []
        }
        // Before the switch for the same reason: what the application says
        // back is not one arm's business, and an arm here is a decision
        // nothing can test.
        if let arriving = Notice(event) {
            notice = Notice.winner(showing: notice, arriving: arriving)
            noticeToken += 1
        }
        switch event {
        case .newMail:
            // Counted as a list change as well as a notification: mail
            // arriving into the folder on screen is exactly the case where
            // the plate has to give way to the list.
            listVersion += 1
            controller.tableView?.reloadData()
            if case let .newMail(account, mailbox, messages) = event {
                arrived(MailArrival(account: account, mailbox: mailbox, messages: messages))
            }
        case .pageReady:
            // The page the table asked for arrived. Redrawing everything is
            // right at this size and wrong at scale; narrowing it to the rows
            // that changed is what `reloadData(forRowIndexes:)` is for and
            // belongs with the rest of the list work.
            controller.tableView?.reloadData()
        case .messageListChanged, .messagesChanged, .messagesRemoved:
            // Both halves: the table redraws its rows, and `listVersion`
            // tells SwiftUI that the *count* moved — which is what decides
            // between the list and the "No messages" plate around it.
            listVersion += 1
            controller.tableView?.reloadData()
        case let .conversationReady(thread):
            // The read that `cursorMoved` started has landed. Checked against
            // what the pane is now showing: a cursor that moved on while the
            // store was reading must not have the old conversation drawn
            // under it.
            if let read = session?.conversation, read.thread == thread, showingThread == thread {
                conversation.show(read)
            }
        case let .cursorMoved(row, message):
            // Every move re-arms, and a move to a row whose page has not
            // arrived cancels: a clock armed against an unknown message would
            // mark whichever one turned up.
            dwell?.cursorMoved(to: message)
            // The table follows the model, never the other way round. `j` and
            // `k` move the cursor behind the boundary -- where the list
            // window, the selection and `aim` all are -- and this is the
            // table catching up with where it ended.
            controller.showCursor(on: row)
            if cursorShowing != message {
                // The panel is about *this* message's tree.
                parts.clear()
                showingParts = false
                // "Once" means this view.
                rendered.clear()
                ccRevealed = []
                // A new message starts at the top. Carrying the anchor over
                // would resume somebody else's place in it.
                readerPage = 0
                readerPageToken += 1
            }
            cursorShowing = message
            openConversation(atRow: row)
        case let .reindexProgress(_, done, total):
            // The settings window asked for this, and it is the only thing
            // that draws it.
            settingsActions.reindexProgressed(done: done, total: total)
        case let .syncProgress(_, done, total):
            syncing = done < total
        case let .connectionChanged(_, state):
            // A connection that has gone means nothing is in flight, whatever
            // the last progress event said.
            if isOffline { syncing = false }
            // **And the reason is kept.** The payload was discarded, so
            // `ConnectionState::Failing`'s reason never reached anything —
            // the footer had three states, none of them "this account cannot
            // sign in", and an expired password read as `idle · synced 40s`
            // for as long as you left it.
            switch state {
            case let .failing(reason):
                failure = reason
                syncing = false
            case .online, .connecting, .offline:
                // Connecting again is not proof it will work, but it is proof
                // the last failure is no longer the current answer.
                failure = nil
            }
        case .mailboxesChanged:
            // The read is above, with the rest of the count-moving events.
            break
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

    /// Build the menu bar from the registry and hang it off `NSApp`.
    ///
    /// Rendered from the same registry the palette and the cheat sheet read
    /// (#657). Accelerators come from the bindings in force where there is a
    /// session to ask and from the built-in defaults before there is one;
    /// none of the items has a key equivalent, because dispatch is the
    /// monitor's.
    ///
    /// Called twice on the way up — once at launch and once when a session
    /// arrives — and `MenuBar` keeps it mounted from there against SwiftUI's
    /// own rebuilds.
    private func installMenuBar() {
        MenuBar.install(
            bindings: { [weak self] command in self?.session?.bindings(for: command) ?? [] },
            // Asked per item, each time a menu opens, against the context
            // that has focus right now — which is what makes a menu item
            // grey out as the keyboard moves between panes. With no session
            // yet, nothing is available: the commands are real but there is
            // nothing for them to act on.
            available: { [weak self] id in
                guard let self else { return false }
                guard let session else {
                    // No session yet — the store is still being unlocked, or
                    // it never opened. Most verbs have nothing to act on, but
                    // the ones this frontend handles itself do not need one:
                    // Settings edits a file, and greying it out while the
                    // Keychain waits leaves the user looking at an
                    // application with nothing enabled and no way to ask why.
                    return Intercepted.all.contains(id)
                }
                return session.isAvailable(id, in: self.context)
            },
            run: { [weak self] id in self?.run(id) }
        )
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
            run: { [weak self] id in self?.run(id) ?? false },
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
    @discardableResult
    func run(_ id: String, on target: Int64? = nil) -> Bool {
        // An overlay taking over means the message is no longer in front of
        // anybody, so a clock in flight must not fire. `DwellClock.stop` is
        // idempotent, so this costs nothing when none is armed.
        if id == Intercepted.palette || id == Intercepted.cheatSheet
            || id == Intercepted.search
        {
            dwell?.stop()
        }
        switch id {
        case Intercepted.palette:
            showingCheatSheet = false
            showingPalette = true
        case Intercepted.cheatSheet:
            showingPalette = false
            showingCheatSheet = true
        case Intercepted.search:
            showingSearch = true
            searchFocusAsks += 1
        case Intercepted.back where showingSearch || session?.isSearching == true:
            // **Escape leaves search, scope and all.** It used to close the
            // field and nothing else, on the strength of a comment saying
            // "`SearchField` restores the scope on its way out" — and
            // `SearchField.leave()` was reachable only from the × button and
            // from submitting an empty query, because the key monitor runs
            // ahead of the responder chain and swallowed `escape` before
            // SwiftUI's `.onKeyPress` ever saw it. So the results stayed, the
            // query stayed, nothing labelled the list as results, and the
            // only way back to the folder was the mouse.
            //
            // `isSearching` as well as `showingSearch`, because the keyboard
            // may have moved on to the list while the results are still up —
            // that is the case GTK's #1474 arm exists for, and `Escape` has
            // to mean the same thing in both.
            showingSearch = false
            if session?.isSearching == true {
                _ = session?.clearSearch()
                listChanged()
                // The list is a folder again, so its context is `List`
                // rather than `Search` -- every other way out of a search
                // already says so.
                searchChanged()
            }
        case Intercepted.cyclePane:
            // The visual order — sidebar, list, reader — and it wraps. A
            // focus order that disagrees with the layout is how a
            // keyboard-first application becomes unusable without a mouse.
            focus(pane.next())
        case Intercepted.cyclePaneBack:
            focus(pane.next(false))
        case Intercepted.focusSidebar:
            focus(.sidebar)
            // The keyboard starts where the folder in view is, so `j` steps
            // on from there rather than back to the top.
            if sidebarCursor == nil {
                folderCursor = mailboxes.first { $0.id == showingMailbox }?.rowId
            }
        case Intercepted.nextFolder:
            return showingResults ? stepScope(by: 1) : stepSidebar(by: 1)
        case Intercepted.prevFolder:
            return showingResults ? stepScope(by: -1) : stepSidebar(by: -1)
        case Intercepted.toggleFolder:
            // A saved search has nothing under it to fold.
            guard case let .folder(row) = sidebarCursor else { return false }
            toggleCollapsed(row)
        case Intercepted.goToInbox:
            return goTo(.inbox)
        case Intercepted.goToDrafts:
            return goTo(.drafts)
        case Intercepted.goToSent:
            return goTo(.sent)
        case Intercepted.goToFlagged:
            return goTo(.flagged)
        case Intercepted.settings:
            // A request the shell turns into `openWindow(id:)`, because only
            // a view can open a window. Not `sendAction(showSettingsWindow:)`
            // -- that reached no handler at all, and because the monitor had
            // already swallowed the key, it also stopped the menu item's own
            // equivalent from running: Postio took `⌘,` and dropped it
            // (#1261). Not the `openSettings` environment value either:
            // reading that from a view inside the `WindowGroup` stops the
            // main window ever completing its first layout, and the
            // application runs, logs and draws nothing (see docs/notes/).
            settingsWindow.raise()
        case Intercepted.toggleSidebar:
            // AppKit's own action rather than a piece of state here: a split
            // view controller owns whether its sidebar is collapsed, and a
            // second opinion in Swift would be one the window ignores.
            NSApp.sendAction(
                #selector(NSSplitViewController.toggleSidebar(_:)), to: nil, from: nil)
        case Intercepted.compose:
            write(session?.newDraft())
        case Intercepted.reply:
            write(replyDraft(all: false, to: target))
        case Intercepted.replyAll:
            write(replyDraft(all: true, to: target))
        case Intercepted.forward:
            guard let session, let message = target ?? cursorShowing else { return false }
            write(session.forwardDraft(message))
        case Intercepted.expandAll:
            conversation.expandAll()
        case Intercepted.toggleFold:
            conversation.toggleFocused()
        case Intercepted.nextInConversation:
            conversation.focusNext()
        case Intercepted.prevInConversation:
            conversation.focusPrevious()
        case Intercepted.back where showingPalette || showingCheatSheet:
            // Escape means "get me out of here", and the innermost "here" is
            // whichever of these is open.
            showingPalette = false
            showingCheatSheet = false
        // -- the settings window's accounts pane ------------------------
        //
        // All seven aim at the row that window's keyboard is on, and a
        // missing cursor is a real answer: falling back to "the first
        // account" would remove somebody's mail on a keystroke aimed at
        // nothing (ADR 0005 Q6c).
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
        // -- saved searches ----------------------------------------------
        case Intercepted.saveSearch:
            // The query on screen, kept. `config.toml` is read at the moment
            // this acts rather than held, because it is hand-edited.
            guard let session, session.isSearching,
                  let query = session.searchQuery
            else { return false }
            editSavedSearch { try saveSearch(path: $0, query: query) }
        case Intercepted.renameSavedSearch:
            guard let row = savedSearches.focused else { return false }
            savedSearches.ask(.rename(key: row.key, from: row.name))
        case Intercepted.deleteSavedSearch:
            // Asked about, never done: PRODUCT.md's rule is that a
            // destructive operation is confirmed or undoable, and taking a
            // `[filters]` entry out of a file nobody kept a copy of cannot
            // be the second.
            guard let row = savedSearches.focused else { return false }
            savedSearches.ask(.confirmDelete(key: row.key, name: row.name))
        case Intercepted.moveSavedSearchUp:
            guard let row = savedSearches.focused else { return false }
            editSavedSearch { try moveSavedSearch(path: $0, key: row.key, direction: .up) }
        case Intercepted.moveSavedSearchDown:
            guard let row = savedSearches.focused else { return false }
            editSavedSearch { try moveSavedSearch(path: $0, key: row.key, direction: .down) }
        case Intercepted.toggleResultOrder:
            // Re-asks the same query the other way round rather than
            // re-sorting the rows on screen: the list is a window over a
            // paged store, and sorting what is resident would order one page.
            guard let session, session.isSearching else { return false }
            session.toggleResultOrder()
            listChanged()
            // The set is the same but its order is not, and the sort label
            // in the toolbar reads through the stamp.
            searchChanged()
        case Intercepted.openParts:
            guard let session, let message = target ?? cursorShowing else { return false }
            parts.show(session.messageParts(message))
            showingParts = true
        case Intercepted.nextPart:
            guard showingParts else { return false }
            parts.step(forward: true)
        case Intercepted.prevPart:
            guard showingParts else { return false }
            parts.step(forward: false)
        case Intercepted.renderPartOnce:
            // No grant is written and no sender is allowed: this loads the
            // one message in front of you, for as long as it is in front of
            // you. `allow_remote_images` is the other gesture.
            guard let message = target ?? cursorShowing else { return false }
            rendered.render(message)
        case Intercepted.savePart:
            guard showingParts else { return false }
            parts.ask(.save)
        case Intercepted.saveAllParts:
            guard showingParts else { return false }
            parts.ask(.saveAll)
        case Intercepted.openPartExternally:
            guard showingParts else { return false }
            parts.ask(.openExternally)
        case Intercepted.openPart:
            guard showingParts else { return false }
            parts.ask(.preview)
        case Intercepted.openMessage:
            // The row is already open — the cursor opens it as it moves — so
            // `Return` is about the *keyboard*: it goes where the message is.
            guard cursorShowing != nil else { return false }
            focus(.reader)
        case Intercepted.prevView:
            // And `h` comes back. Not "close the message": the pane is not a
            // drill-in on either platform, so there is nothing to close.
            focus(.list)
        case Intercepted.viewOriginal:
            guard let message = target ?? cursorShowing else { return false }
            toggleOriginal(message)
        case Intercepted.scrollReaderDown, Intercepted.scrollReaderUp:
            // Only the single-message pane pages this way: it is one
            // document, so the shared anchors are in it and a fragment jump
            // lands exactly. The conversation pane is a stack of documents
            // inside a `ScrollView`, and the honest answer there is to *not*
            // take the key — AppKit pages a scroll view on `space` itself,
            // and a frontend claiming the key to do nothing is the bug this
            // return value exists for.
            guard showingThread == nil, cursorShowing != nil else { return false }
            readerPage = readerPageAfter(
                current: readerPage,
                forward: id == Intercepted.scrollReaderDown
            )
            readerPageToken += 1
        default:
            // A compose window in front gets first refusal on the composer's
            // own verbs — and only the window that has the keyboard, because
            // several can be open and Send in one must not send another.
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

    /// Which of the reading pane's scroll anchors it is on.
    ///
    /// A hardened web view has no scroll-by-amount call, so paging is a jump
    /// between the anchors the shared document lays down — see
    /// `ReaderPaging`. Reset when the message changes, or `space` on a new
    /// message would resume somebody else's place in it.
    /// Which messages are being shown as their sender wrote them.
    ///
    /// Per message and per view — see `OriginalView`. Here rather than in the
    /// pane because `⌘O` is a command, and a command cannot reach an
    /// `@State`: that is exactly why the key did nothing while the `⋯` menu
    /// item beside it worked.
    private(set) var original = OriginalView()

    /// A message's parts, when the panel is open.
    ///
    /// The cursor inside it is the only thing this side decides; see
    /// `PartsModel`.
    let parts = PartsModel()

    /// Whether the parts panel is showing.
    ///
    /// **It moves the key context with it**, like the palette and the search
    /// field: `Context::Parts` is where `j`, `k`, `s`, `S`, `x`, `H` and
    /// `Return` mean what the panel needs them to mean. Without this they
    /// would keep resolving as the list underneath — and `s`, `x` and
    /// `Return` all do something there, so pressing Save over an attachment
    /// would have acted on a message instead.
    var showingParts = false {
        didSet {
            guard showingParts != oldValue else { return }
            paneContext = showingParts ? .parts : contextOf(pane)
        }
    }

    /// Which messages are drawing what the reader held back.
    ///
    /// Per message and per view — see `RenderedOnce`. Here rather than in the
    /// conversation view because `H` is a command, and the blocked-images
    /// notice's own button presses the same thing.
    private(set) var rendered = RenderedOnce()

    /// Show `message` as its sender wrote it, or stop.
    ///
    /// A method rather than a settable property: `original` is
    /// `private(set)` so the only ways to change it are this and the two
    /// places that clear it when the pane shows something else.
    /// Which messages have their `Cc` list open, outside any conversation.
    ///
    /// `ConversationModel` holds this per conversation; the single-message
    /// pane has no conversation to hold it, and the disclosure is still a
    /// thing a person opened. Per message, and reset with the pane.
    private(set) var ccRevealed: Set<Int64> = []

    /// Open or close `message`'s `Cc` list in the single-message pane.
    func toggleCc(_ message: Int64) {
        if ccRevealed.contains(message) {
            ccRevealed.remove(message)
        } else {
            ccRevealed.insert(message)
        }
    }

    func toggleOriginal(_ message: Int64) {
        original.toggle(message)
    }

    private(set) var readerPage: UInt32 = 0

    /// Bumped whenever a page turn is asked for.
    ///
    /// The *number* is not enough on its own: paging down at the last anchor
    /// leaves it where it was, and a view watching the value would not
    /// redraw — which is fine, but paging up from 0 twice has the same shape
    /// and the token keeps the two honest.
    private(set) var readerPageToken = 0

    /// Open a compose window for `draft`, or say why there is none.
    ///
    /// A missing draft is not a silent no-op: on a fresh install there is no
    /// account to write from, and a `⌘N` that appeared to do nothing is the
    /// shape of bug this port has produced three times.
    private func write(_ draft: DraftFfi?) {
        guard let draft else {
            NSSound.beep()
            return
        }
        compose.open(draft)
    }

    /// Open a composer on a `mailto:` link. `false` when there is nothing to
    /// open one from, which the caller says out loud rather than swallowing.
    ///
    /// The draft is the boundary's — recipients, subject and body assembled
    /// there, so GTK gets the same behaviour from the same code — and only
    /// the window is this frontend's.
    func write(mailto: Mailto) -> Bool {
        guard let session, let draft = session.mailtoDraft(mailto) else { return false }
        compose.open(draft)
        return true
    }

    /// A reply to `target`, or to the message the cursor is on.
    ///
    /// The cursor, not the selection: `PRODUCT.md` §9 keeps them apart, and
    /// replying to twelve marked messages is not a thing.
    ///
    /// `target` is what a **per-message** surface passes — the conversation
    /// pane draws a verb bar under every open message, and those must answer
    /// the message they are under rather than the list's cursor. Without it,
    /// Reply under message three of an eight-message thread composed a reply
    /// to the thread's representative message: the wrong recipient, silently.
    private func replyDraft(all: Bool, to target: Int64? = nil) -> DraftFfi? {
        guard let session, let message = target ?? cursorShowing else { return nil }
        return session.replyDraft(to: message, all: all)
    }

    /// Close whatever overlay is open, and put the keyboard back in the list.
    func dismissOverlays() {
        showingPalette = false
        showingCheatSheet = false
        showingSearch = false
        paneContext = contextOf(pane)
    }

    /// Redraw the list against whatever scope the boundary is now on.
    ///
    /// Called after a search runs or is cleared. The generation the boundary
    /// answered with is what the window is already on; this is only the table
    /// catching up with a row count that changed underneath it.
    func listChanged() {
        guard case let .open(controller) = state else { return }
        listVersion += 1
        controller.tableView?.reloadData()
    }

    /// The conversation the pane has been asked for, so a read that lands
    /// late can be dropped rather than drawn.
    private(set) var showingThread: Int64?

    /// Show the conversation the row at `row` belongs to.
    ///
    /// Every row in a folder stands for a conversation (ADR 0015), and a
    /// message row in a search result still belongs to one — so this is what
    /// landing on a row means in both. A row with no thread leaves the pane
    /// showing the message itself, which is the honest answer for mail that
    /// threading could not place.
    private func openConversation(atRow row: UInt32?) {
        guard let session, let row, let thread = session.row(at: row)?.thread else {
            showingThread = nil
            // **And empty the pane.** Clearing the token alone left the last
            // conversation drawn under the new selection — and because the
            // shell picks the conversation pane on `conversation != nil`,
            // which latched true after the first `show`, the single-message
            // branch beside it was unreachable from then on. Somebody else's
            // mail, under a row that is not theirs, looking like an answer.
            conversation.clear()
            original.clear()
            return
        }
        guard thread != showingThread else { return }
        showingThread = thread
        // A different conversation is a different view, and the grants were
        // about the last one's messages.
        original.clear()
        session.openConversation(thread)
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
        openConversation(atRow: row)
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
        dwell?.stop()
        dwell = nil
        reachability.stop()
        session?.shutdown()
        session = nil
    }
}
