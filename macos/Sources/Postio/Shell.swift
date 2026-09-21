import PostioFFI
import PostioKit
import SwiftUI

/// The three panes: folders, messages, and the message.
///
/// `NavigationSplitView` rather than the nested `GtkPaned` the Linux frontend
/// uses. `postio-gtk/src/shell.rs` explains why it avoided
/// `AdwNavigationSplitView` — it needed the pane position to be a savable
/// number — and that reasoning is GTK's. Here the native idiom brings sidebar
/// collapse, a full-height sidebar and toolbar unification for free, and
/// column widths persist through `SceneStorage`.
///
/// The layout is the same three panes on both platforms because
/// `docs/PRODUCT.md` §9 says so, not because the widgets happen to match.
struct Shell: View {
    @State private var engine: Engine
    /// Only a view can open a window, so the shell is where a `settings`
    /// command becomes one (#1261).
    @Environment(\.openWindow) private var openWindow
    /// Which sidebar row is picked.
    ///
    /// A `SidebarRowId` rather than a mailbox id: three of the rows are
    /// queries with no id of their own, so an `Int64?` made Flagged, Snoozed
    /// and the Outbox all the same row. See `SidebarRowId`.
    @State private var selectedFolder: SidebarRowId?
    @State private var showing: Int64?
    /// The folder that was open. Application state rather than window state —
    /// it is about the account, not the window — but stored with the scene
    /// because that is where a scene's restoration lives.
    ///
    /// `Int` rather than `Int64`: `SceneStorage` has no overload for the
    /// latter, and a mailbox id fits either on every platform Postio builds
    /// for. The conversion is at the two edges rather than in the type, so
    /// nothing else has to know.
    @SceneStorage("openFolder") private var openFolder: Int?

    /// The saved search being renamed, and the name being typed for it.
    @State private var renamingKey: String?
    @State private var renamedTo = ""
    /// The saved search being deleted, and what it is called — the
    /// confirmation names it, because a dialog that says "are you sure?" and
    /// nothing else is one people learn to dismiss without reading.
    @State private var deletingKey: String?
    @State private var deletingName = ""

    init(engine: Engine) {
        _engine = State(initialValue: engine)
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selectedFolder) {
                if engine.mailboxes.isEmpty {
                    Text("No folders yet")
                        .foregroundStyle(.secondary)
                        .font(.callout)
                } else {
                    // The special-use folders first, in the order the
                    // boundary gave them — Inbox at the top, one row per
                    // role. Nothing is sorted here; see `Engine.specialFolders`.
                    Section("Favorites") {
                        ForEach(engine.specialFolders, id: \.rowId) { folder in
                            // A special-use folder stands for its role and
                            // is drawn flat, children or not: the Favorites
                            // section is one row per role, and its tree is
                            // under "On My Mac".
                            FolderRow(
                                folder: folder,
                                children: { _ in [] },
                                collapsed: expansion
                            )
                        }
                    }
                    // A query somebody wrote down, beside the folders it
                    // searches. Not selectable rows: see `SavedSearchRows`.
                    SavedSearchRows(searches: engine.savedSearches) { search in
                        engine.open(search)
                    }
                    // Then the account's own folders, each account a group
                    // and each folder with its children under it. The tree is
                    // rebuilt here from the flat list's parent ids —
                    // flattening it for display would turn a tidy account
                    // into slash-separated strings.
                    if !engine.folderRoots.isEmpty {
                        Section("On My Mac") {
                            ForEach(engine.accountsWithFolders, id: \.id) { account in
                                AccountFolders(
                                    address: account.address,
                                    roots: engine.folderRoots.filter { $0.account == account.id },
                                    children: { engine.children(of: $0) },
                                    collapsed: expansion
                                )
                                // The account is a heading, not a folder.
                                // Inside a `List(selection:)` every row is
                                // selectable unless it says otherwise, so
                                // clicking the address highlighted it as
                                // though mail had been opened, and nothing
                                // was.
                                .selectionDisabled()
                            }
                        }
                    }
                }
            }
            // Postio's ramp rather than AppKit's sidebar material. The
            // material is a blue-grey, and the reader's paper is a pure
            // neutral — put one inside the other and the neutral reads
            // maroon (#1588). `scrollContentBackground` is what lets the
            // colour underneath show at all.
            .scrollContentBackground(.hidden)
            .background(Color(nsColor: AppSurface.sidebar))
            .safeAreaInset(edge: .bottom, spacing: 0) { footer }
            .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 320)
            .accessibilityLabel(Pane.sidebar.label)
            .onTapGesture { engine.focus(.sidebar) }
            .onChange(of: selectedFolder) { _, picked in
                guard let picked,
                      let row = engine.mailboxes.first(where: { $0.rowId == picked })
                else { return }
                engine.open(row)
                // Only a real folder is worth remembering across launches. A
                // view is a question about mail that may not be there next
                // time, and reopening onto an empty Outbox reads as a broken
                // restore.
                if !row.isView { openFolder = Int(row.id) }
            }
            // The folder list arrives after the session opens, so the folder
            // to reopen can only be chosen once there is something to choose
            // among -- and it has to survive the list arriving empty first.
            .onChange(of: engine.mailboxes.count) { _, _ in restoreFolder() }
            .onAppear { restoreFolder() }
        } content: {
            messages
                .background(Color(nsColor: AppSurface.background))
                .navigationSplitViewColumnWidth(min: 280, ideal: 360, max: 560)
        } detail: {
            reader
                .background(Color(nsColor: AppSurface.background))
                .onTapGesture { engine.focus(.reader) }
        }
        .toolbar {
            // No sidebar-toggle item here: `NavigationSplitView` puts one at
            // the leading edge itself, and adding a second drew two identical
            // buttons an inch apart. `toggle_sidebar` runs the same AppKit
            // action the built-in one does, so the key and the button are one
            // behaviour with one control.
            // Icons only, in the canvas' order, every one a registry command
            // with a tooltip that names the key it is bound to.
            ToolbarItemGroup {
                ForEach(ToolbarPlan.items, id: \.command) { item in
                    Button {
                        engine.run(item.command)
                    } label: {
                        Image(systemName: item.symbol)
                    }
                    .help(
                        ToolbarPlan.tooltip(for: item) { command in
                            engine.session?.bindings(for: command) ?? []
                        }
                    )
                    .accessibilityLabel(item.title)
                    .disabled(!available(item.command))
                }
            }
            // The most prominent control in the window, at the trailing edge
            // (canvas screen 25). On screen always: before this, search was a
            // keystroke with nothing to announce it (#1260).
            ToolbarItem(placement: .primaryAction) {
                if let session = engine.session {
                    SearchField(
                        session: session,
                        reload: {
                            engine.listChanged()
                            // A list of results resolves keys as
                            // `Context::Search`; a list of a mailbox does
                            // not. Running or clearing a search does not
                            // move the keyboard, so nothing else notices.
                            engine.searchChanged()
                        },
                        dismiss: { engine.dismissOverlays() },
                        wantsFocus: Binding(
                            get: { engine.showingSearch },
                            set: { engine.showingSearch = $0 }
                        )
                    )
                    .frame(minWidth: 220, idealWidth: 320)
                }
            }
        }
        // No `navigationTitle`. The canvas' title bar is empty: this
        // application's name belongs in the menu bar and the About window,
        // and a window that announces which program it is spends a line of
        // chrome telling you something you knew when you opened it.
        // The palette, over everything, with the keyboard in it. `context`
        // follows so the resolver answers for the surface that actually has
        // focus -- a palette that still resolved keys as the list would
        // archive mail while somebody typed a command's name.
        .overlay {
            if engine.showingPalette, let session = engine.session {
                Color.black.opacity(0.12)
                    .ignoresSafeArea()
                    .onTapGesture { engine.dismissOverlays() }
                Palette(
                    session: session,
                    context: .list,
                    run: { engine.run($0) },
                    dismiss: { engine.dismissOverlays() }
                )
                .onAppear { engine.paneContext = .palette }
            }
        }
        .sheet(isPresented: $engine.showingCheatSheet) {
            if let session = engine.session {
                CheatSheet(
                    session: session,
                    context: .list,
                    dismiss: { engine.dismissOverlays() }
                )
            }
        }
        // A half-typed sequence, shown while it waits. `g` on its own is a
        // second of the application looking like it ignored a key, and the
        // resolver reports the pending chords precisely so it does not have
        // to be.
        // `PRODUCT.md` §18: ≤100 ms or absent, and Reduce Motion is honoured.
        // `Motion.current` is read here rather than cached, because the
        // preference can change while Postio is running and a cached copy
        // would keep animating for somebody who had just asked it to stop.
        .animation(.easeOut(duration: Motion.current), value: engine.pendingChord)
        .animation(.easeOut(duration: Motion.current), value: engine.showingPalette)
        .animation(.easeOut(duration: Motion.current), value: engine.noticeToken)
        // What Postio said back. Bottom-*leading*, so it never lands under
        // the pending-chord hint at the other corner: both are transient and
        // both can be up at once — a `g` half-typed while a send fails.
        // The parts panel: a sheet rather than a pane, because it is about
        // one message and is opened to do one thing. `p` opens it and `Esc`
        // closes it, which is the Done button's `.cancelAction`.
        .sheet(isPresented: $engine.showingParts) {
            if let session = engine.session, let message = engine.cursorShowing {
                PartsPanel(
                    session: session,
                    message: message,
                    model: engine.parts,
                    // What the reader held back for *this* message. The
                    // panel's note and its "Render once" both turn on the
                    // counts, and the counts are the notice's.
                    held: engine.heldBack(for: message),
                    renderOnce: { engine.run(Intercepted.renderPartOnce, on: message) },
                    dismiss: { engine.showingParts = false }
                )
            }
        }
        // Two questions a command cannot ask: renaming needs a field, and
        // deleting needs a confirmation. Both are worded by the boundary
        // (ADR 0019 Q6) — two platforms writing their own sentence for a
        // destructive verb is two products.
        .onChange(of: engine.savedSearches.wishToken) { _, _ in
            switch engine.savedSearches.wish {
            case let .rename(key, from):
                renamingKey = key
                renamedTo = from
            case let .confirmDelete(key, name):
                deletingKey = key
                deletingName = name
            case nil:
                break
            }
        }
        .alert(
            savedSearchRenamePrompt().title,
            isPresented: Binding(get: { renamingKey != nil }, set: { if !$0 { renamingKey = nil } })
        ) {
            TextField("", text: $renamedTo)
            Button(savedSearchRenamePrompt().confirm) {
                if let key = renamingKey { engine.renameSavedSearch(key, to: renamedTo) }
                renamingKey = nil
            }
            Button(savedSearchRenamePrompt().cancel, role: .cancel) { renamingKey = nil }
        } message: {
            if let body = savedSearchRenamePrompt().body { Text(body) }
        }
        .alert(
            savedSearchDeletePrompt().title,
            isPresented: Binding(get: { deletingKey != nil }, set: { if !$0 { deletingKey = nil } })
        ) {
            Button(savedSearchDeletePrompt().confirm, role: .destructive) {
                if let key = deletingKey { engine.deleteSavedSearch(key) }
                deletingKey = nil
            }
            Button(savedSearchDeletePrompt().cancel, role: .cancel) { deletingKey = nil }
        } message: {
            Text(savedSearchDeletePrompt().body ?? "\(deletingName) goes for good.")
        }
        .overlay(alignment: .bottomLeading) { noticeBanner }
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
        // A notification click. The engine has already switched the list to
        // the folder; the sidebar selection and the reader follow so that all
        // three panes agree about what is being shown.
        // A keystroke moves the cursor without touching the table, so the
        // reading pane follows the engine rather than the click.
        .onChange(of: engine.cursorShowing) { _, message in
            if let message { showing = message }
        }
        // A count rather than a flag: two `⌘,` presses are two openings, and
        // `onChange` compares values (see `WindowRequest`).
        .onChange(of: engine.settingsWindow) { _, request in
            guard request.wasRaised else { return }
            openWindow(id: request.id)
        }
        // A compose window per draft: the store holds the draft, the shell
        // is what can open a window for it.
        .onChange(of: engine.compose.request) { _, request in
            guard request.wasRaised, let draft = engine.compose.requested else { return }
            openWindow(id: WindowId.compose, value: draft)
        }
        .onChange(of: engine.requestedToken) { _, _ in
            guard let requested = engine.requested else { return }
            // A notification names a real folder, so the folder row is the
            // one to pick — never a view, which holds no mail of its own.
            selectedFolder = engine.mailboxes
                .first { $0.id == requested.mailbox && !$0.isView }?
                .rowId
            // A burst names no message -- "3 new messages" does not pick one --
            // so it opens the folder and leaves the cursor where the folder's
            // own selection puts it.
            if let message = requested.message { showing = message }
        }
    }

    /// What Postio last said back about something you asked it to do.
    ///
    /// Not a dialog and not a log line: a sentence where the eye already is,
    /// that goes on its own, with an Undo beside it when there is something
    /// to take back. `Notice` decides how long it stays and whether it offers
    /// that button; this only draws it, which is the split that keeps the
    /// decision testable at all — nothing can test a view here.
    @ViewBuilder
    private var noticeBanner: some View {
        if let notice = engine.notice {
            HStack(spacing: PostioTokens.space2) {
                Image(
                    systemName: notice.isAlarming
                        ? "exclamationmark.triangle.fill" : "checkmark.circle"
                )
                .foregroundStyle(notice.isAlarming ? Color.red : Color.secondary)
                Text(notice.message)
                    .lineLimit(2)
                if notice.offersUndo {
                    // The registry's `undo`, so this button and `u` are one
                    // command. A button with an undo of its own would be a
                    // second undo stack.
                    Button("Undo") {
                        engine.run(Notice.undoCommand)
                        engine.dismissNotice()
                    }
                    .buttonStyle(.link)
                }
            }
            .padding(.horizontal, PostioTokens.space3)
            .padding(.vertical, PostioTokens.space2)
            .background(.regularMaterial, in: .rect(cornerRadius: 8))
            .padding(12)
            .transition(.opacity)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(notice.message)
            // A sentence that appears in a corner is one a screen-reader user
            // never hears otherwise.
            .accessibilityAddTraits(.updatesFrequently)
            // Its own lifetime, keyed on the *token* rather than the notice:
            // *Archived* twice in a row is two notices, and the second has to
            // restart the clock rather than inherit what was left of the
            // first one's.
            .task(id: engine.noticeToken) {
                try? await Task.sleep(nanoseconds: UInt64(notice.seconds * 1_000_000_000))
                guard !Task.isCancelled else { return }
                engine.dismissNotice()
            }
        }
    }

    /// The line under the folders: a state dot and a sentence.
    ///
    /// Redrawn on a timer because the sentence ages — "synced 40s" is only
    /// true for a second. Every fifteen seconds rather than every second: the
    /// line is glanced at, and a footer that repaints at 1 Hz is a window
    /// that never settles.
    private var footer: some View {
        TimelineView(.periodic(from: .now, by: 15)) { timeline in
            HStack(spacing: PostioTokens.space2) {
                Circle()
                    .fill(
                        SidebarFooter.isResting(
                            offline: engine.isOffline,
                            syncing: engine.syncing,
                            failure: engine.failure
                        )
                            ? Color.secondary
                            // An account that cannot sign in is the one thing
                            // on this line somebody has to act on, so it is
                            // the one colour that is not the accent.
                            : engine.failure == nil
                                ? Color(nsColor: PostioTokens.colorAccent)
                                : Color.red
                    )
                    .frame(width: 7, height: 7)
                Text(
                    SidebarFooter.status(
                        mailboxes: engine.mailboxes,
                        offline: engine.isOffline,
                        syncing: engine.syncing,
                        failure: engine.failure,
                        now: timeline.date
                    )
                )
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(engine.failure == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(Color.red))
                .lineLimit(1)
                Spacer()
            }
            .padding(.horizontal, PostioTokens.space4)
            .padding(.vertical, PostioTokens.space3)
            .background(.bar)
            .accessibilityElement(children: .combine)
        }
    }

    /// Whether a toolbar button should be live.
    ///
    /// The same question the palette's filter and the menu ask, so the three
    /// cannot disagree about what this view can do.
    private func available(_ command: String) -> Bool {
        engine.session?.isAvailable(command, in: engine.context) ?? false
    }

    /// Reopen the folder that was open, or the inbox if it is gone.
    /// A folder's disclosure, as a binding onto the engine's own state.
    ///
    /// Inverted on purpose: `DisclosureGroup` asks whether it is *expanded*
    /// and the engine records what is *collapsed*, because the walk's
    /// question is "what can I not see" and an empty set is the ordinary
    /// case — everything open.
    private func expansion(_ folder: MailboxFfi) -> Binding<Bool> {
        Binding(
            get: { !engine.collapsedFolders.contains(folder.rowId) },
            set: { engine.setCollapsed(folder.rowId, !$0) }
        )
    }

    private func restoreFolder() {
        guard selectedFolder == nil, !engine.mailboxes.isEmpty else { return }
        let folder = WindowState.folderToOpen(
            remembered: openFolder.map(Int64.init),
            among: engine.mailboxes
        )
        selectedFolder = folder
            .flatMap { id in engine.mailboxes.first { $0.id == id && !$0.isView } }
            .map(\.rowId)
    }

    @ViewBuilder
    private var messages: some View {
        switch engine.state {
        case .opening:
            // Not decoration. The store's key comes from the login Keychain,
            // and macOS raises its prompt in front of whatever window the
            // asking application has -- so this *is* the window the prompt
            // appears over. Before #1146 there was none, and the application
            // sat in the Dock drawing nothing while being asked a question
            // nobody could connect to it.
            ContentUnavailableView {
                Label("Unlocking your mail", systemImage: "lock")
            } description: {
                Text("Postio is asking the Keychain for this store's key.")
            }
        case let .open(controller):
            if engine.rowCount == 0 {
                // Empty is a state, not a blank. A list showing nothing and a
                // list that failed to load look identical otherwise, and only
                // one of them is worth waiting for.
                //
                // **Which** empty is the boundary's to say. A search that
                // matched nothing has a row count of zero like an empty
                // mailbox does, and this branch drew "This store has no mail
                // in it yet." over both — over a mailbox holding thousands, a
                // confident false statement about somebody's own mail. That
                // is ADR 0005 Q10's worked example: you search for an
                // invoice, find nothing, and conclude it does not exist.
                if let plate = engine.session?.emptyPlate {
                    ContentUnavailableView(
                        plate.title,
                        systemImage: "magnifyingglass",
                        description: Text(plate.detail)
                    )
                } else {
                    ContentUnavailableView(
                        "No messages",
                        systemImage: "tray",
                        description: Text("This store has no mail in it yet.")
                    )
                }
            } else {
                VStack(spacing: 0) {
                    // No search strip here any more: the field lives in the
                    // toolbar, where the canvas puts it and where it is
                    // visible without a keystroke.
                    // "12 selected", when there is a selection to say it
                    // about. From the model, which knows the answer for a
                    // whole-view selection without enumerating it -- a count
                    // taken from ids on this side could not draw the one case
                    // that most needs a count.
                    if let summary = engine.selectionSummary {
                        HStack {
                            Text(summary)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                            Spacer()
                        }
                        .padding(.horizontal, 12)
                        .padding(.vertical, 6)
                        .background(.quaternary.opacity(0.4))
                    }
                    MessageListView(controller: controller)
                }
                .accessibilityLabel(Pane.list.label)
                .onTapGesture { engine.focus(.list) }
                .onAppear {
                    controller.onCursorChanged = { message in
                        showing = message
                        // The engine needs it too, and for a different
                        // reason: the reader draws what the cursor is on,
                        // and `aim` decides what a verb with nothing
                        // marked acts on. Without this `a` archives
                        // nothing at all.
                        engine.cursorMoved(to: message)
                    }
                    // A click moves the cursor, and the boundary has to be
                    // told *where* -- `j` afterwards steps from there.
                    controller.onCursorRowChanged = { row in
                        engine.cursorClicked(row: row)
                    }
                    // A hover action or a context-menu item acts on the row it
                    // was asked on, not on wherever the cursor happens to be.
                    // Moving the cursor there first is what makes that true
                    // without a second, targeted dispatch path: the verb then
                    // runs exactly as the keystroke would, undo included.
                    controller.onRowAction = { command, row in
                        controller.showCursor(on: UInt32(row))
                        engine.cursorClicked(row: UInt32(row))
                        engine.cursorMoved(to: controller.messageAt(row: row))
                        engine.run(command)
                    }
                }
            }
        case let .unavailable(reason):
            ContentUnavailableView {
                Label("The engine did not open", systemImage: "exclamationmark.triangle")
            } description: {
                Text(reason)
            }
        }
    }

    @ViewBuilder
    private var reader: some View {
        if case .opening = engine.state {
            // Nothing to say yet, and "no message selected" would be a claim
            // about a store that has not been opened.
            Color.clear
        } else if let session = engine.session, engine.conversation.conversation != nil {
            // The whole conversation, stacked (ADR 0015 Q4). The list stays a
            // list: there is no drill-in, and nothing about this pane is a
            // second place mail is listed.
            ConversationView(
                session: session,
                model: engine.conversation,
                // The message the verb was drawn under travels with it: a
                // per-message bar that answered the *list's* cursor replied
                // to the wrong message in any thread longer than one.
                run: { engine.run($0, on: $1) },
                showingOriginal: { engine.original.isOn($0) },
                showingImages: { engine.rendered.isOn($0) },
                toggleOriginal: { engine.toggleOriginal($0) }
            )
        } else if let session = engine.session, let showing, let row = session.rowFor(showing) {
            // A message that threading could not place belongs to no
            // conversation, and the honest thing to draw is the message —
            // with its sender, its subject, its date and its verbs, which
            // this pane did not have (#1585). It is the conversation's own
            // `ExpandedMessage`, so there is one answer to "who is this
            // from" rather than two that drift; only *Collapse* goes, since
            // there is no conversation to fold it into.
            ScrollView {
                ExpandedMessage(
                    session: session,
                    row: row,
                    isLatest: true,
                    showingCc: engine.ccRevealed.contains(showing),
                    showingOriginal: engine.original.isOn(showing),
                    showingImages: engine.rendered.isOn(showing),
                    collapsible: false,
                    collapse: {},
                    toggleCc: { engine.toggleCc(showing) },
                    toggleOriginal: { engine.toggleOriginal(showing) },
                    run: { engine.run($0, on: $1) },
                    openSettings: { engine.run(Intercepted.settings) }
                )
                .padding(.horizontal, PostioTokens.space4)
                .padding(.vertical, PostioTokens.space4)
            }
        } else {
            ContentUnavailableView(
                "No message selected",
                systemImage: "envelope",
                description: Text("Choose a message to read it.")
            )
        }
    }
}
