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
                    // Roots first, each with its children under it. The tree
                    // is rebuilt here from the flat list's parent ids —
                    // flattening it for display would turn a tidy account into
                    // slash-separated strings.
                    ForEach(engine.folderRoots, id: \.id) { folder in
                        FolderRow(folder: folder, children: engine.children(of: folder.id))
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 320)
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
                .onTapGesture { engine.context = .reader }
        }
        .navigationTitle("Postio")
        // A half-typed sequence, shown while it waits. `g` on its own is a
        // second of the application looking like it ignored a key, and the
        // resolver reports the pending chords precisely so it does not have
        // to be.
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
        .onChange(of: engine.requestedToken) { _, _ in
            guard let requested = engine.requested else { return }
            selectedFolder = requested.mailbox
            // A burst names no message -- "3 new messages" does not pick one --
            // so it opens the folder and leaves the cursor where the folder's
            // own selection puts it.
            if let message = requested.message { showing = message }
        }
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
                MessageListView(controller: controller)
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
        if let session = engine.session, let showing {
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
