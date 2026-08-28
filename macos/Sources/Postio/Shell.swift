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
    @State private var selectedFolder: String? = "Inbox"

    init(engine: Engine) {
        _engine = State(initialValue: engine)
    }

    var body: some View {
        NavigationSplitView {
            List(selection: $selectedFolder) {
                Section("Postio") {
                    Label("Inbox", systemImage: "tray").tag("Inbox")
                    Label("Flagged", systemImage: "flag").tag("Flagged")
                }
            }
            .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 320)
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
        ContentUnavailableView(
            "No message selected",
            systemImage: "envelope",
            description: Text("The reading pane arrives with the reader.")
        )
    }
}
