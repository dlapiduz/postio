import AppKit
import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI

/// The main window: Focus's inbox. The toolbar is AppKit's (`MainToolbar`) because the command bar
/// drops from its search field, whose width it controls.
struct MainWindow: View {
    let engine: Engine
    /// Only a view can open a window: `settings` and `compose` become one here (#1261).
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            if case let .refused(model) = engine.state {
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
        // A count, not a flag: two `⌘,` presses are two openings, and `onChange` compares values
        // (`WindowRequest`).
        .onChange(of: engine.settingsWindow) { _, request in
            guard request.wasRaised else { return }
            openWindow(id: request.id)
        }
    }

    private var inbox: some View {
        ZStack {
            inboxColumn
            // The list stays under Filtered so its scroll and cursor survive the way back.
            if let filtered = engine.filtered, filtered.isOpen {
                FilteredView(model: filtered, scope: engine.states.footer)
            }
            // The inbox stays under the results, keeping its scroll, cursor and selection.
            if let results = engine.results, results.isOpen, let table = engine.resultsTable,
               let query = engine.searchQuery
            {
                ResultsPane(engine: engine, results: results, query: query, table: table)
            }
            if let contacts = engine.contactsView, contacts.isOpen {
                ContactsScreen(model: contacts)
            }
            if let io = engine.contactsIo, let sheet = io.sheet {
                ContactsIoSheetView(sheet: sheet, model: io)
            }
        }
        .animation(nil, value: engine.filtered?.isOpen)
        .animation(nil, value: engine.results?.isOpen)
        .animation(nil, value: engine.contactsView?.isOpen)
        .background(HistorySwipeInstaller(engine: engine))
        // A child window rather than an overlay (`CommandBarPanel`).
        .background(MainToolbarInstaller(engine: engine))
        .sheet(isPresented: Binding(
            get: { engine.bannerRepairAccount != nil },
            set: { if !$0 { engine.bannerRepair.cancel() } }
        )) {
            if let account = engine.bannerRepairAccount {
                PasswordSheet(account: account, repair: engine.bannerRepair, session: engine.session)
            }
        }
        .animation(.easeOut(duration: Motion.current), value: engine.pendingChord)
        .animation(.easeOut(duration: Motion.current), value: engine.noticeToken)
        .animation(.easeOut(duration: Motion.current), value: engine.actionBarWords != nil)
        .animation(.easeOut(duration: Motion.current), value: engine.states.banner)
        .animation(.easeOut(duration: Motion.current), value: engine.focus.toastToken)
        .animation(.easeOut(duration: Motion.current), value: engine.focus.toast == nil)
        .overlay(alignment: .bottomLeading) { NoticeBanner(engine: engine) }
        .overlay {
            if let words = engine.keyMap.words {
                KeyMapPanel(words: words, dismiss: { engine.keyMapDismissed() })
                    .transition(.opacity)
            }
        }
        .animation(.easeOut(duration: Motion.current), value: engine.keyMap.isOpen)
        .overlay(alignment: .bottom) {
            if let toast = engine.focus.toast {
                UndoPill(
                    toast: toast, token: engine.focus.toastToken, undoCap: engine.undoCap,
                    undo: { engine.run(Notice.undoCommand) },
                    dismiss: { engine.focus.dismissToast(token: $0) }
                )
                .padding(.bottom, engine.barHeightUnderToast)
            }
        }
        .overlay(alignment: .bottomTrailing) {
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
            HeaderStrip(
                words: engine.stripWords,
                placeAnchor: PlacesAnchor { engine.placesAnchor = $0 }
            ) { engine.run($0) }
            if let banner = engine.states.banner {
                BannerStrip(words: BannerStripWords(banner)) { engine.run($0) }
                    .transition(.opacity)
            }
            list
            if let words = engine.actionBarWords {
                ActionBar(words: words) { engine.run($0) }
                    .transition(.opacity)
            }
            if let footer = engine.states.footer {
                ScopeFooter(words: ScopeFooterWords(footer))
            }
        }
    }

    @ViewBuilder
    private var list: some View {
        switch engine.state {
        case .opening:
            ContentUnavailableView {
                Label("Unlocking your mail", systemImage: "lock")
            } description: {
                Text("Postio is asking the Keychain for this store's key.")
            }
        case .open:
            if let table = engine.focusTable {
                // The table stays under the empty page so the keyboard stays on the list, not on
                // the first key view (Inbox ▾, which Space would press).
                FocusListView(table: table)
                    .overlay {
                        if let page = engine.states.empty {
                            EmptyInbox(words: EmptyInboxWords(page)) { engine.run($0) }
                        }
                    }
            } else if let page = engine.states.empty {
                EmptyInbox(words: EmptyInboxWords(page)) { engine.run($0) }
            }
        case .refused:
            EmptyView()
        }
    }
}

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
            // The table stays under the page, empty, so it keeps the keyboard.
            let none = engine.noResults.flatMap { $0.isOpen ? $0 : nil }
            if none == nil {
                TimelineView(
                    countLine: results.countLine, subLine: results.subLine, months: results.months,
                    hint: results.timelineHint, step: results.timelineStep,
                    onMonths: { [weak session = engine.session] first, last in
                        session?.focusSearchMonths(first, last)
                    })
            }
            if results.isFiles, let header = results.filesHeader, let grid = engine.filesGrid {
                FilesHeaderView(header: header)
                FilesGridRepresentable(grid: grid)
            } else if results.isPeople, none == nil {
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

private struct NoticeBanner: View {
    let engine: Engine

    var body: some View {
        if let notice = engine.notice {
            HStack(spacing: 8) {
                Image(systemName: notice.isAlarming ? "exclamationmark.triangle.fill" : "checkmark.circle")
                    .foregroundStyle(notice.isAlarming ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                Text(notice.message).lineLimit(2)
                if notice.offersUndo {
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

/// The command bar drops from the toolbar's search field, which grows to 860 while it is up, so the
/// toolbar is AppKit's. No `.toolbar` in the SwiftUI tree, so SwiftUI has none to put back.
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
        // `window` is nil until the view joins a hierarchy.
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

@MainActor
final class MainToolbar: NSObject, NSToolbarDelegate, NSSearchFieldDelegate {
    private let engine: Engine
    private let toolbar = NSToolbar(identifier: "PostioFocusMain")
    private let searchBox = ToolbarSearchBox()
    private var field: BarSearchField { searchBox.field }
    private weak var window: NSWindow?
    private var barOpen = false

    static let compose = NSToolbarItem.Identifier("postio.compose")
    static let sync = NSToolbarItem.Identifier("postio.sync")
    static let search = NSToolbarItem.Identifier("postio.search")
    static let back = NSToolbarItem.Identifier("postio.results.back")
    static let query = NSToolbarItem.Identifier("postio.results.query")
    static let save = NSToolbarItem.Identifier("postio.results.save")

    static let inboxItems: [NSToolbarItem.Identifier] = [compose, .flexibleSpace, sync, search]
    static let resultsItems: [NSToolbarItem.Identifier] = [back, query, save]

    private var showingResults = false
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
        engine.searchField = field
        engine.keycapsChanged = { [weak self] in self?.respell() }
        engine.barShown = { [weak self] shown in self?.grow(shown) }
        engine.resultsShown = { [weak self] shown in self?.showResults(shown) }
        layoutSearch()
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

    private func showResults(_ shown: Bool) {
        guard shown != showingResults else { return }
        showingResults = shown
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

    private func layoutQuery() {
        guard showingResults, let window else { return }
        let taken = Self.lights + backView.fittingSize.width + saveView.fittingSize.width
            + Self.gaps + CommandBarGeometry.edge
        queryBox.boxWidth = max(window.frame.width - taken, 240)
    }

    private static let lights: CGFloat = 86
    private static let gaps: CGFloat = 32

    /// The sync label gives way while grown: the field covers its place, and a toolbar that cannot
    /// fit both would shrink the field.
    private func grow(_ shown: Bool) {
        if showingResults {
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

    private func layoutSearch() {
        guard !showingResults else { return }
        searchBox.open(barOpen, windowWidth: window?.frame.width ?? 0)
    }

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
            field.onFocus = { [weak self] in self?.engine.searchFieldFocused() }
            searchBox.placeholder = engine.session?.focusSearchPlaceholder() ?? ""
            searchBox.restingPlaceholder = engine.session?.focusSearchRestingPlaceholder() ?? ""
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

    private func composeTip() -> String {
        KeyCapSpelling.cap(engine.session?.binding(for: WritingCommand.compose))
            .map { "Compose \u{b7} \($0)" } ?? "Compose"
    }

    private func respell() {
        backView.rootView = ResultsBackButton(engine: engine)
        saveView.rootView = SaveSearchButton(engine: engine)
        engine.savePresenter?.anchor = saveView
        searchBox.restingCap = KeyCapSpelling.cap(engine.session?.binding(for: BarCommand.palette))
        searchBox.openCap = KeyCapSpelling.cap("Escape")
        if let placeholder = engine.session?.focusSearchPlaceholder() {
            searchBox.placeholder = placeholder
        }
        if let resting = engine.session?.focusSearchRestingPlaceholder() {
            searchBox.restingPlaceholder = resting
        }
        for item in toolbar.items where item.itemIdentifier == Self.compose {
            item.toolTip = composeTip()
        }
    }

    func controlTextDidEndEditing(_ obj: Notification) {
        engine.searchFieldLeft()
    }

    func controlTextDidChange(_ obj: Notification) {
        let text = (obj.object as? NSSearchField)?.stringValue ?? field.stringValue
        searchBox.textChanged()
        engine.searchFieldTyped(text)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        engine.searchFieldCommand(selector)
    }
}

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

/// The words are `postio_ui::focus_state::sync_label`'s; a Swift copy is the drift the FFI event
/// ends.
private struct SyncLabelView: View {
    let engine: Engine

    var body: some View {
        if let label = engine.states.syncLabel {
            ToolbarSyncLabel(label: label)
        }
    }
}
