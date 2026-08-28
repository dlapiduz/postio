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
                if let folder { engine.open(mailbox: folder) }
            }
        } content: {
            messages
                .navigationSplitViewColumnWidth(min: 280, ideal: 360, max: 560)
        } detail: {
            reader
        }
        .navigationTitle("Postio")
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
                    .onAppear { controller.onCursorChanged = { showing = $0 } }
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
