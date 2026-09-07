import PostioFFI
import PostioKit
import SwiftUI

/// The application.
///
/// Useless on purpose, for now. It shows what came back through the boundary,
/// which is the only thing worth asserting at this stage: every other link in
/// the chain — cargo, the bindings generator, the module map, the linker, the
/// bundle — fails in its own way and none of them is covered by anything else.
@main
struct PostioApp: App {
    @State private var engine = Engine()
    // Resolved by `postio_config::paths`, so this edits the file the rest of
    // Postio reads rather than a second opinion about where settings live.
    @State private var settings = SettingsStore(path: (try? settingsPath()) ?? "")
    @Environment(\.scenePhase) private var phase
    // SwiftUI's `onOpenURL` only fires for a scene that already exists, and a
    // `mailto:` click is the ordinary way Postio gets launched in the first
    // place. The delegate sees both.
    @NSApplicationDelegateAdaptor(URLHandler.self) private var urls

    var body: some Scene {
        WindowGroup("Postio") {
            Shell(engine: engine)
                .background(WindowConfigurator())
                // `[ui].theme`, not the system's, when the file says so.
                .preferredColorScheme(engine.colorScheme)
        }
        .onChange(of: phase) { _, now in
            // Orderly rather than at process exit: the store is SQLCipher, and
            // dropping an engine as the process ends is exactly when
            // libcrypto goes away underneath a thread still encrypting a page.
            if now == .background { engine.shutdown() }
        }
        .defaultSize(width: 1100, height: 700)
        .windowToolbarStyle(.unified)
        // Size and position across launches. `SceneStorage` handles the split
        // widths; the frame is `NSWindow`'s own autosave, which is the only
        // thing that survives a window being closed and reopened rather than
        // the app being quit.
        .windowResizability(.contentSize)

        // A real window, not an overlay on the main one: `⌘,` has opened one
        // on this platform since Mac OS X 10.0, and ADR 0019 Q1 rejected the
        // cheap port precisely for having "the wrong window chrome". Canvas
        // 3f's contract -- one store, no OK/Cancel, nav that jumps, validity
        // along the foot -- is kept in full; only the frame is different.
        //
        // `Window` rather than SwiftUI's `Settings` scene, because the scene
        // can only be opened by a selector that reached no handler here
        // (#1261) and by a menu item this application does not draw --
        // `MenuBar` builds the application menu from the registry. A window
        // with an id is opened by `openWindow`, which is a call rather than a
        // hope.
        Window("Settings", id: WindowId.settings) {
            SettingsPaneView(
                store: settings,
                accounts: engine.accounts,
                mailboxes: engine.mailboxes,
                actions: engine.settingsActions,
                session: engine.session
            )
                .preferredColorScheme(engine.colorScheme)
        }
        .defaultSize(width: 900, height: 560)
        .windowResizability(.contentSize)

        // Compose: its own window, several at once, each in the Window menu
        // (canvas screen 26). `WindowGroup` rather than `Window` for exactly
        // that reason — a `Window` is a singleton, and writing two messages
        // at once is ordinary.
        WindowGroup(id: WindowId.compose, for: Int64.self) { $draft in
            if let draft, let model = engine.compose.model(draft), let session = engine.session {
                ComposeView(
                    session: session,
                    model: model,
                    close: { engine.compose.close(draft) }
                )
                .preferredColorScheme(engine.colorScheme)
                .navigationTitle(model.title)
            }
        }
        .defaultSize(width: 640, height: 520)
    }
}
