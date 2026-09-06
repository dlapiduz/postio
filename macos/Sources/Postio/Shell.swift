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
    @State private var selectedFolder: Int64?
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
                        ForEach(engine.specialFolders, id: \.id) { folder in
                            FolderRow(folder: folder, children: [])
                        }
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
                                    children: { engine.children(of: $0) }
                                )
                            }
                        }
                    }
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) { footer }
            .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 320)
            .accessibilityLabel(Pane.sidebar.label)
            .onTapGesture { engine.focus(.sidebar) }
            .onChange(of: selectedFolder) { _, folder in
                guard let folder else { return }
                engine.open(mailbox: folder)
                openFolder = Int(folder)
            }
            // The folder list arrives after the session opens, so the folder
            // to reopen can only be chosen once there is something to choose
            // among -- and it has to survive the list arriving empty first.
            .onChange(of: engine.mailboxes.count) { _, _ in restoreFolder() }
            .onAppear { restoreFolder() }
        } content: {
            messages
                .navigationSplitViewColumnWidth(min: 280, ideal: 360, max: 560)
        } detail: {
            reader
                .onTapGesture { engine.focus(.reader) }
        }
        .toolbar {
            // The sidebar toggle first, where every Mac window puts it. The
            // registry command runs the same AppKit action, so the button and
            // `\` are one behaviour rather than two.
            ToolbarItem(placement: .navigation) {
                Button {
                    engine.run("toggle_sidebar")
                } label: {
                    Image(systemName: "sidebar.left")
                }
                .help("Show or hide the sidebar")
                .accessibilityLabel("Show or hide the sidebar")
            }
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
                            engine.session?.binding(for: command)
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
                        reload: { engine.listChanged() },
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
                .onAppear { engine.context = .palette }
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
        .onChange(of: engine.requestedToken) { _, _ in
            guard let requested = engine.requested else { return }
            selectedFolder = requested.mailbox
            // A burst names no message -- "3 new messages" does not pick one --
            // so it opens the folder and leaves the cursor where the folder's
            // own selection puts it.
            if let message = requested.message { showing = message }
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
                        SidebarFooter.isResting(offline: engine.isOffline, syncing: engine.syncing)
                            ? Color.secondary
                            : Color(nsColor: PostioTokens.colorAccent)
                    )
                    .frame(width: 7, height: 7)
                Text(
                    SidebarFooter.status(
                        mailboxes: engine.mailboxes,
                        offline: engine.isOffline,
                        syncing: engine.syncing,
                        now: timeline.date
                    )
                )
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
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
    private func restoreFolder() {
        guard selectedFolder == nil, !engine.mailboxes.isEmpty else { return }
        selectedFolder = WindowState.folderToOpen(
            remembered: openFolder.map(Int64.init),
            among: engine.mailboxes
        )
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
                ContentUnavailableView(
                    "No messages",
                    systemImage: "tray",
                    description: Text("This store has no mail in it yet.")
                )
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
                run: { engine.run($0) }
            )
        } else if let session = engine.session, let showing {
            // A message that threading could not place belongs to no
            // conversation, and the honest thing to draw is the message.
            // Remote images blocked. `PRODUCT.md`'s "nothing leaves this
            // machine that the user did not ask for" starts at the tracking
            // pixel, and per-sender allowing is its own work.
            ReaderView(session: session, message: showing, remoteImages: .blocked)
        } else {
            ContentUnavailableView(
                "No message selected",
                systemImage: "envelope",
                description: Text("Choose a message to read it.")
            )
        }
    }
}
