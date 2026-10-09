import AppKit
import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI

/// The main window: Focus's inbox (specs/009-focus-macos US1, screens 01
/// and 02).
///
/// A unified toolbar -- compose on the left, the sync label and the search
/// field on the right -- then the header strip, then the list. No sidebar,
/// no reading pane: an email opens in its own window (FR-011, later), and
/// the list is the whole of the window.
///
/// The toolbar is AppKit's (`MainToolbar`), because the command bar drops
/// from its search field (FR-013), whose width it controls; the strip is
/// SwiftUI over `HeaderStripWords`; the list is `FocusListTable` (R10).
struct MainWindow: View {
    let engine: Engine
    /// Only a view can open a window, so this is where a `settings` or
    /// `compose` command becomes one (#1261).
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            // A fresh install is the wizard, not an empty inbox: with no
            // account there is nothing for the list to show (canvas 09).
            if case let .refused(model) = engine.state {
                // The store would not open (T100): why, and the way forward,
                // in place of the whole inbox -- with no store there is no
                // strip to count and no list to show.
                StoreRefusalPage(model: model)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if let firstRun = engine.firstRun, engine.accounts.isEmpty {
                FirstRunView(
                    model: firstRun,
                    otherWays: { engine.run(Intercepted.addAccount) },
                    finished: { engine.settingsActions.accountAdded?() }
                )
            } else {
                inbox
            }
        }
        // The inbox fills the window: without a size of its own the table
        // reports almost none, and the window shrank to a strip of rows.
        .frame(minWidth: 800, idealWidth: 1440, minHeight: 500, idealHeight: 900)
        // A count rather than a flag: two `⌘,` presses are two openings, and
        // `onChange` compares values (see `WindowRequest`).
        .onChange(of: engine.settingsWindow) { _, request in
            guard request.wasRaised else { return }
            openWindow(id: request.id)
        }
    }

    private var inbox: some View {
        ZStack {
            inboxColumn
            // Filtered (`g f`, screen 21) in the list's place, header and
            // all, while the controller has it up. The list stays under it,
            // so its scroll and its cursor are where they were on the way
            // back.
            if let filtered = engine.filtered, filtered.isOpen {
                FilteredView(model: filtered)
            }
            // The results (specs/010-focus-search step 3): a mode of the
            // window, in the inbox's place, the inbox kept under it -- its
            // scroll, cursor and selection are where they were on the way
            // back, as Filtered's are.
            if let results = engine.results, results.isOpen, let table = engine.resultsTable,
               let query = engine.searchQuery
            {
                ResultsPane(engine: engine, results: results, query: query, table: table)
            }
        }
        .animation(nil, value: engine.filtered?.isOpen)
        .animation(nil, value: engine.results?.isOpen)
        // ⌘[ and ⌘] by trackpad: the swipe is reported as the command.
        .background(HistorySwipeInstaller(engine: engine))
        // The command bar is a panel dropping from the toolbar's field
        // (`CommandBarPanel`, T085), a child window rather than an overlay.
        .background(MainToolbarInstaller(engine: engine))
        // "Update password…" from the sign-in banner (screen 19).
        .sheet(isPresented: Binding(
            get: { engine.bannerRepairAccount != nil },
            set: { if !$0 { engine.bannerRepair.cancel() } }
        )) {
            if let account = engine.bannerRepairAccount {
                PasswordSheet(account: account, repair: engine.bannerRepair, session: engine.session)
            }
        }
        // `PRODUCT.md` §18: ≤100 ms or absent, and Reduce Motion is honoured.
        .animation(.easeOut(duration: Motion.current), value: engine.pendingChord)
        .animation(.easeOut(duration: Motion.current), value: engine.noticeToken)
        .animation(.easeOut(duration: Motion.current), value: engine.actionBarWords != nil)
        .animation(.easeOut(duration: Motion.current), value: engine.states.banner)
        .animation(.easeOut(duration: Motion.current), value: engine.focus.toastToken)
        .animation(.easeOut(duration: Motion.current), value: engine.focus.toast == nil)
        .overlay(alignment: .bottomLeading) { NoticeBanner(engine: engine) }
        // The key map (`?`, screen 20): a panel over the dimmed list, opened
        // and closed by the controller; a click outside closes it too.
        .overlay {
            if let words = engine.keyMap.words {
                KeyMapPanel(words: words, dismiss: { engine.keyMapDismissed() })
                    .transition(.opacity)
            }
        }
        .animation(.easeOut(duration: Motion.current), value: engine.keyMap.isOpen)
        // What a verb did, with Undo while the stack can take it back: the
        // controller's toast, as the pill at the bottom centre (T093,
        // screen 15). Above the action bar when there is one.
        .overlay(alignment: .bottom) {
            if let toast = engine.focus.toast {
                UndoPill(
                    toast: toast, token: engine.focus.toastToken, undoCap: engine.undoCap,
                    undo: { engine.run(Notice.undoCommand) },
                    dismiss: { engine.focus.dismissToast(token: $0) }
                )
                .padding(.bottom, engine.actionBarWords == nil || engine.filtered?.isOpen == true ? 0 : ActionBar.height)
            }
        }
        .overlay(alignment: .bottomTrailing) {
            // A half-typed sequence, shown while it waits: `g` on its own is
            // otherwise a second of the application ignoring a key.
            if let pending = engine.pendingChord {
                Text(pending)
                    .font(.system(.body, design: .monospaced))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(.regularMaterial, in: .rect(cornerRadius: 6))
                    .padding(12)
                    .transition(.opacity)
                    .accessibilityLabel("Waiting for the rest of \(pending)")
            }
        }
    }

    private var inboxColumn: some View {
        VStack(spacing: 0) {
            // Inbox ▾ is what the folders popover hangs from (T086).
            HeaderStrip(
                words: engine.stripWords,
                placeAnchor: PlacesAnchor { engine.placesAnchor = $0 }
            ) { engine.run($0) }
            // First sync, offline, a refused password (screens 17 to 19):
            // one full-width strip under the header strip, as the
            // controller words it. Never in the way: the list stays live.
            if let banner = engine.states.banner {
                BannerStrip(words: BannerStripWords(banner)) { engine.run($0) }
                    .transition(.opacity)
            }
            list
            // While anything is selected (screen 01): the count, the verbs
            // with their keys, and the selection's own keys.
            if let words = engine.actionBarWords {
                ActionBar(words: words) { engine.run($0) }
                    .transition(.opacity)
            }
        }
    }

    @ViewBuilder
    private var list: some View {
        switch engine.state {
        case .opening:
            // The window the Keychain's prompt appears over (#1146).
            ContentUnavailableView {
                Label("Unlocking your mail", systemImage: "lock")
            } description: {
                Text("Postio is asking the Keychain for this store's key.")
            }
        case .open:
            if let page = engine.states.empty {
                // Empty is a state, not a blank (screen 16): the controller's
                // page, with its next digest and the shortcuts that lead
                // somewhere, in the list's place until it says the list is
                // back.
                EmptyInbox(words: EmptyInboxWords(page)) { engine.run($0) }
            } else if let table = engine.focusTable {
                FocusListView(table: table)
            }
        case .refused:
            // Drawn in the inbox's place, above.
            EmptyView()
        }
    }
}

/// The results view's body, under the toolbar (design §3.2-3.5, screens 06
/// and 07): the filter bar, the timeline, the grouped rows and the footer.
/// Every word is the controller's; the toolbar's back, query and save are
/// `MainToolbar`'s.
private struct ResultsPane: View {
    let engine: Engine
    let results: ResultsModel
    let query: SearchQueryModel
    let table: ResultsTable

    var body: some View {
        VStack(spacing: 0) {
            FilterBarView(
                query: query, tabs: results.tabs, order: results.order, sortable: results.sortable,
                words: engine.searchWords,
                pickTab: { results.pick($0) }, pickOrder: { results.pick($0) },
                anchor: { kind in
                    guard let presenter = engine.filterPopoverPresenter else { return AnyView(EmptyView()) }
                    return AnyView(FilterPopoverAnchor(kind: kind, presenter: presenter))
                })
            // Nothing found (step 7, screen 13): no timeline, and the page
            // over the rows' place. The table stays under it, empty, so it
            // keeps the keyboard it had and has it when rows come back.
            let none = engine.noResults.flatMap { $0.isOpen ? $0 : nil }
            if none == nil {
                TimelineView(
                    countLine: results.countLine, subLine: results.subLine, months: results.months,
                    hint: results.timelineHint, step: results.timelineStep,
                    onMonths: { [weak session = engine.session] first, last in
                        session?.focusSearchMonths(first, last)
                    })
            }
            // The Files tab (step 9): its header and grid in the rows' place.
            if results.isFiles, let header = results.filesHeader, let grid = engine.filesGrid {
                FilesHeaderView(header: header)
                FilesGridRepresentable(grid: grid)
            } else if results.isPeople, none == nil {
                // The People tab (step 10): its list in the rows' place.
                PeopleListView(model: results)
            } else {
                ResultsTableRepresentable(table: table)
                    .overlay {
                        if let none {
                            NoResultsView(model: none)
                                .background(Color(nsColor: .textBackgroundColor))
                        }
                    }
            }
            SearchFooter(
                hints: results.footerHints, right: results.footerRight, checked: engine.resultsChecked,
                bulk: results.bulk, selectAll: results.selectAll)
        }
        .background(Color(nsColor: .textBackgroundColor))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(results.countLine)
    }
}

/// A failure Postio has to report: a sentence where the eye already is,
/// gone on its own. `Notice` decides; this draws. Completions, undos and
/// refusals are the controller's toast in this window (`UndoPill`),
/// so only `Notice.shownBesideFocusToast` reaches here.
private struct NoticeBanner: View {
    let engine: Engine

    var body: some View {
        if let notice = engine.notice {
            HStack(spacing: 8) {
                Image(systemName: notice.isAlarming ? "exclamationmark.triangle.fill" : "checkmark.circle")
                    .foregroundStyle(notice.isAlarming ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                Text(notice.message).lineLimit(2)
                if notice.offersUndo {
                    // The registry's `undo`, so this button and the key are
                    // one command with one stack.
                    Button("Undo") {
                        engine.run(Notice.undoCommand)
                        engine.dismissNotice()
                    }
                    .buttonStyle(.link)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(.regularMaterial, in: .rect(cornerRadius: 8))
            .padding(12)
            .transition(.opacity)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(notice.message)
            .accessibilityAddTraits(.updatesFrequently)
            .task(id: engine.noticeToken) {
                try? await Task.sleep(nanoseconds: UInt64(notice.seconds * 1_000_000_000))
                guard !Task.isCancelled else { return }
                engine.dismissNotice()
            }
        }
    }
}

/// Reports the trackpad's back and forward on the window this view is in
/// as ⌘[ and ⌘] would (specs/010-focus-search T073, `HistorySwipe`).
private struct HistorySwipeInstaller: NSViewRepresentable {
    let engine: Engine

    final class Coordinator {
        var swipe: HistorySwipe?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        let coordinator = context.coordinator
        let engine = engine
        DispatchQueue.main.async {
            guard let window = view.window, coordinator.swipe == nil else { return }
            let swipe = HistorySwipe(window: window) { command in engine.run(command) }
            swipe.start()
            coordinator.swipe = swipe
        }
        return view
    }

    func updateNSView(_: NSView, context _: Context) {}
}

// MARK: - the toolbar

/// Puts `MainToolbar` on the window this view is in, once it is in one.
///
/// The command bar drops from the toolbar's search field (FR-013), which
/// grows to 860 while it is up, so the toolbar is AppKit's. Nothing in the
/// SwiftUI tree declares a `.toolbar`, so SwiftUI has none of its own to
/// put back over this one.
private struct MainToolbarInstaller: NSViewRepresentable {
    let engine: Engine

    final class Coordinator {
        var toolbar: MainToolbar?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        let coordinator = context.coordinator
        let engine = engine
        // `window` is nil until the view joins a hierarchy. One hop.
        DispatchQueue.main.async {
            guard let window = view.window else { return }
            let toolbar = coordinator.toolbar ?? MainToolbar(engine: engine)
            coordinator.toolbar = toolbar
            toolbar.install(in: window)
        }
        return view
    }

    func updateNSView(_: NSView, context _: Context) {}
}

/// The unified toolbar (FR-010): compose on the left, a flexible space,
/// the sync label, and the search field with its ⌘K keycap.
@MainActor
final class MainToolbar: NSObject, NSToolbarDelegate, NSSearchFieldDelegate {
    private let engine: Engine
    private let toolbar = NSToolbar(identifier: "PostioFocusMain")
    /// The inbox's search field (T054): Postio's own, whose width it sets.
    private let searchBox = ToolbarSearchBox()
    private var field: BarSearchField { searchBox.field }
    private weak var window: NSWindow?
    /// Whether the bar is up over the inbox's field.
    private var barOpen = false

    static let compose = NSToolbarItem.Identifier("postio.compose")
    static let sync = NSToolbarItem.Identifier("postio.sync")
    static let search = NSToolbarItem.Identifier("postio.search")
    // The results' (specs/010-focus-search §3.1): ‹ Inbox, the query, Save.
    static let back = NSToolbarItem.Identifier("postio.results.back")
    static let query = NSToolbarItem.Identifier("postio.results.query")
    static let save = NSToolbarItem.Identifier("postio.results.save")

    /// The inbox's items, and the results'.
    static let inboxItems: [NSToolbarItem.Identifier] = [compose, .flexibleSpace, sync, search]
    static let resultsItems: [NSToolbarItem.Identifier] = [back, query, save]

    /// Whether the results' items are up.
    private var showingResults = false
    /// The results' query box, kept while the inbox's items are up.
    private lazy var queryBox: ResultsQueryBox = makeQueryBox()
    private lazy var backView = NSHostingView(rootView: ResultsBackButton(engine: engine))
    private lazy var saveView = NSHostingView(rootView: SaveSearchButton(engine: engine))

    init(engine: Engine) {
        self.engine = engine
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
    }

    func install(in window: NSWindow) {
        guard self.window !== window else { return }
        self.window = window
        window.toolbarStyle = .unified
        window.titleVisibility = .hidden
        if window.toolbar !== toolbar { window.toolbar = toolbar }
        // SwiftUI rebuilds what it owns when a scene updates; if that ever
        // takes the toolbar back, put this one back rather than lose search.
        NotificationCenter.default.addObserver(
            forName: NSWindow.didUpdateNotification, object: window, queue: .main
        ) { [weak self] note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated {
                guard let self, window.toolbar !== self.toolbar else { return }
                window.toolbar = self.toolbar
            }
        }
        // The field is the command bar's (T085): the engine puts the
        // controller's words in it and gives it the keyboard.
        engine.searchField = field
        engine.keycapsChanged = { [weak self] in self?.respell() }
        engine.barShown = { [weak self] shown in self?.grow(shown) }
        engine.resultsShown = { [weak self] shown in self?.showResults(shown) }
        NotificationCenter.default.addObserver(
            forName: NSWindow.didResizeNotification, object: window, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.layoutQuery()
                self?.layoutSearch()
            }
        }
        respell()
    }

    /// The results are up, or gone: the toolbar's items follow (§3.1).
    /// The search field the bar edits in is the query box's while they are
    /// up, so `/` drops the dropdown from it.
    private func showResults(_ shown: Bool) {
        guard shown != showingResults else { return }
        showingResults = shown
        // The inbox's field comes back at rest, with the sync label.
        barOpen = false
        searchBox.open(false, windowWidth: window?.frame.width ?? 0)
        let items = shown ? Self.resultsItems : Self.inboxItems
        while !toolbar.items.isEmpty { toolbar.removeItem(at: 0) }
        for (index, identifier) in items.enumerated() {
            toolbar.insertItem(withItemIdentifier: identifier, at: index)
        }
        engine.queryBox = shown ? queryBox : nil
        engine.searchField = shown ? queryBox.editor : field
        if shown {
            engine.redrawQuery()
            layoutQuery()
        } else {
            queryBox.editing = false
        }
        respell()
    }

    private func makeQueryBox() -> ResultsQueryBox {
        let box = ResultsQueryBox()
        box.field.onFocus = { [weak self] in self?.engine.queryFieldFocused() }
        box.field.onType = { [weak self] text in self?.engine.queryFieldTyped(text) }
        box.field.onRemove = { [weak self] token in self?.engine.searchQuery?.remove(token) }
        box.editor.delegate = self
        box.editor.sendsSearchStringImmediately = false
        box.editor.sendsWholeSearchString = true
        box.editor.onFocus = { [weak self] in self?.engine.searchFieldFocused() }
        return box
    }

    /// The query box fills what the toolbar leaves it: the window less the
    /// window's buttons, ‹ Inbox, Save search and the gaps between.
    private func layoutQuery() {
        guard showingResults, let window else { return }
        let taken = Self.lights + backView.fittingSize.width + saveView.fittingSize.width
            + Self.gaps + CommandBarGeometry.edge
        queryBox.boxWidth = max(window.frame.width - taken, 240)
    }

    /// The window's buttons and the toolbar's leading inset.
    private static let lights: CGFloat = 86
    /// The toolbar's spacing between the results' three items.
    private static let gaps: CGFloat = 32

    /// The field grows leftward to 860 while the bar is up, its right edge
    /// where it was, and goes back after (specs/010-focus-search T054).
    /// The sync label gives way while it is grown: the field covers its
    /// place (screen 01), and a toolbar that cannot fit both would shrink
    /// the field instead.
    private func grow(_ shown: Bool) {
        if showingResults {
            // The query is edited as text in the box's own place.
            queryBox.editing = shown
            return
        }
        guard shown != barOpen else { return }
        barOpen = shown
        let syncAt = toolbar.items.firstIndex { $0.itemIdentifier == Self.sync }
        if shown, let syncAt {
            toolbar.removeItem(at: syncAt)
        } else if !shown, syncAt == nil,
                  let searchAt = toolbar.items.firstIndex(where: { $0.itemIdentifier == Self.search })
        {
            toolbar.insertItem(withItemIdentifier: Self.sync, at: searchAt)
        }
        layoutSearch()
    }

    /// The inbox's field at its width for the window as it is now.
    private func layoutSearch() {
        guard !showingResults else { return }
        searchBox.open(barOpen, windowWidth: window?.frame.width ?? 0)
        window?.contentView?.superview?.layoutSubtreeIfNeeded()
    }

    // MARK: NSToolbarDelegate

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [Self.compose, .flexibleSpace, Self.sync, Self.search]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        Self.inboxItems + Self.resultsItems
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        switch identifier {
        case Self.compose:
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.image = NSImage(systemSymbolName: "square.and.pencil", accessibilityDescription: "Compose")
            item.label = "Compose"
            item.target = self
            item.action = #selector(compose)
            item.toolTip = composeTip()
            return item
        case Self.sync:
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.view = NSHostingView(rootView: SyncLabelView(engine: engine))
            item.label = "Sync"
            return item
        case Self.search:
            let item = searchBox.item(identifier)
            // A click into it opens the bar, as `/` does.
            field.onFocus = { [weak self] in self?.engine.searchFieldFocused() }
            searchBox.placeholder = engine.session?.focusSearchPlaceholder() ?? ""
            field.delegate = self
            field.sendsSearchStringImmediately = false
            field.sendsWholeSearchString = true
            if !showingResults { engine.searchField = field }
            respell()
            return item
        case Self.back:
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.view = backView
            item.label = engine.searchWords.back
            return item
        case Self.query:
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.view = queryBox
            item.label = "Search"
            return item
        case Self.save:
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.view = saveView
            item.label = engine.searchWords.save
            engine.savePresenter?.anchor = saveView
            return item
        default:
            return nil
        }
    }

    @objc private func compose() {
        engine.run(WritingCommand.compose)
    }

    /// "Compose · c", as `postio_ui::focus_row::compose_tooltip` says it.
    private func composeTip() -> String {
        KeyCapSpelling.cap(engine.session?.binding(for: WritingCommand.compose))
            .map { "Compose \u{b7} \($0)" } ?? "Compose"
    }

    /// The keycap in the field and the compose tip, from the bindings in
    /// force: at install, when a session opens, and when `[keys]` changes.
    private func respell() {
        backView.rootView = ResultsBackButton(engine: engine)
        saveView.rootView = SaveSearchButton(engine: engine)
        engine.savePresenter?.anchor = saveView
        searchBox.restingCap = KeyCapSpelling.cap(engine.session?.binding(for: BarCommand.palette))
        // Escape closes the bar: the field's own key, not a command.
        searchBox.openCap = KeyCapSpelling.cap("Escape")
        if let placeholder = engine.session?.focusSearchPlaceholder() {
            searchBox.placeholder = placeholder
        }
        for item in toolbar.items where item.itemIdentifier == Self.compose {
            item.toolTip = composeTip()
        }
    }

    // MARK: NSSearchFieldDelegate

    func controlTextDidEndEditing(_ obj: Notification) {
        engine.searchFieldLeft()
    }

    func controlTextDidChange(_ obj: Notification) {
        // The inbox's field, or the results' query box's editor.
        let text = (obj.object as? NSSearchField)?.stringValue ?? field.stringValue
        searchBox.textChanged()
        engine.searchFieldTyped(text)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        engine.searchFieldCommand(selector)
    }
}

/// ‹ Inbox with its Esc keycap (§3.1): Back, as Escape is.
private struct ResultsBackButton: View {
    let engine: Engine

    var body: some View {
        Button { engine.run(ResultsCommand.back) } label: {
            HStack(spacing: 6) {
                Image(systemName: "chevron.left").font(.system(size: 12, weight: .semibold))
                Text(engine.searchWords.back).font(.system(size: 13, weight: .semibold))
                if let cap = KeyCapSpelling.cap(engine.session?.binding(for: ResultsCommand.back)) {
                    KeyCap(cap)
                }
            }
            .padding(.leading, 4)
            .padding(.trailing, 6)
            .frame(height: 28)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fixedSize()
        .accessibilityLabel(engine.searchWords.back)
    }
}

/// Save search with its ⌘S keycap (§3.1): what ⌘S runs, and what the Save
/// popover hangs from (specs/010-focus-search T100).
private struct SaveSearchButton: View {
    let engine: Engine

    var body: some View {
        Button { engine.run(BarCommand.saveSearch) } label: {
            HStack(spacing: 6) {
                Image(systemName: "bookmark").font(.system(size: 12))
                Text(engine.searchWords.save).font(.system(size: 13, weight: .semibold))
                if let cap = KeyCapSpelling.cap(engine.session?.binding(for: BarCommand.saveSearch)) {
                    KeyCap(cap)
                }
            }
            .padding(.leading, 10)
            .padding(.trailing, 8)
            .frame(height: 30)
            .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(.separator, lineWidth: 1))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fixedSize()
    }
}

/// The sync label: a symbol and the controller's words ("Synced 16:09",
/// "Syncing 12,408 of 18,204", "Offline", "Sync failed"), as
/// `FocusSyncLabel` last said them. Nothing until it has said anything: the
/// words are `postio_ui::focus_state::sync_label`'s, and a Swift copy of
/// them is the drift the FFI event exists to end.
private struct SyncLabelView: View {
    let engine: Engine

    var body: some View {
        if let label = engine.states.syncLabel {
            HStack(spacing: 5) {
                Image(systemName: label.symbol).font(.system(size: 11))
                Text(label.text).font(.system(size: 12).monospacedDigit())
            }
            .foregroundStyle(label.isAlarming ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
            .fixedSize()
            .accessibilityElement(children: .combine)
        }
    }
}
