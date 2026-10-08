import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI

/// The application: Postio Focus on the Mac (specs/009-focus-macos).
///
/// The main window is Focus's inbox (`MainWindow`); settings and compose
/// are windows of their own.
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
            MainWindow(engine: engine)
                .background(WindowConfigurator())
                // `[ui].theme`, not the system's, when the file says so.
                .preferredColorScheme(engine.colorScheme)
                // The delegate is created before the engine has a session, so
                // it is handed a closure rather than the engine: a `mailto:`
                // that arrives during launch finds whatever is true when it
                // is clicked, not what was true when the delegate was built.
                .onAppear {
                    urls.write = { engine.write(mailto: $0) }
                    // Quitting is where the orderly shutdown belongs. The
                    // delegate is the only thing that hears it.
                    urls.stop = { engine.shutdown() }
                }
        }
        .onChange(of: phase) { _, now in
            // **Not on `.background`.** That is what `⌘W`, `⌘H` and
            // minimising all produce on this platform — the gesture a Mac
            // user makes to leave a mail client *running* — and ending the
            // session there stopped every sync, every IDLE connection and
            // every new-mail notification. With no way back, because the
            // session is opened once: reopening a window found a full mailbox
            // drawing "No messages" and no keyboard.
            //
            // The shutdown is still orderly; it has moved to
            // `applicationWillTerminate`, which is where quitting is.
            // `SessionLifetime` is the rule, and carries the rest of it.
            if SessionLifetime.shouldEnd(on: SessionPhase(now)) { engine.shutdown() }
        }
        // Screen 01's size: the Mac pack draws the inbox at 1440 × 900.
        .defaultSize(width: 1440, height: 900)
        .windowToolbarStyle(.unified)
        // Size and position across launches: the frame is `NSWindow`'s own
        // autosave, which is the only thing that survives a window being
        // closed and reopened rather than the app being quit.
        .windowResizability(.contentMinSize)

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
            // A view of its own, and the reason matters: see `SettingsWindow`.
            SettingsWindow(engine: engine, settings: settings)
        }
        .defaultSize(width: 900, height: 560)
        .windowResizability(.contentSize)

        // Compose: its own window, several at once, each in the Window menu
        // (canvas screen 26). `WindowGroup` rather than `Window` for exactly
        // that reason — a `Window` is a singleton, and writing two messages
        // at once is ordinary.
        WindowGroup(id: WindowId.compose, for: Int64.self) { $draft in
            ComposeWindow(engine: engine, draft: draft)
        }
        .defaultSize(width: 640, height: 520)
    }
}

/// `ScenePhase` as `SessionLifetime` asks about it.
///
/// The mapping is here rather than in `PostioKit` because `ScenePhase` is
/// SwiftUI's and this is the target that has SwiftUI in it. `terminating` has
/// no `ScenePhase` at all — it arrives from `applicationWillTerminate`, which
/// is the whole reason `SessionPhase` is its own type.
extension SessionPhase {
    init(_ phase: ScenePhase) {
        switch phase {
        case .active: self = .active
        case .inactive: self = .inactive
        case .background: self = .background
        // A phase a future macOS invents is not a reason to stop collecting
        // mail. `inactive` is the conservative reading: the application is
        // still running.
        @unknown default: self = .inactive
        }
    }
}

/// The Settings window's content, as a view of its own.
///
/// **Not inline in `PostioApp.body`, and this is the whole point of it.**
/// SwiftUI evaluates every scene to build the menu bar -- a `Window` scene
/// gets its own item in the Window menu -- so whatever a scene's content
/// reads off the engine becomes a reason to rebuild the *menus*. Inline,
/// this read `engine.mailboxes`, which moves with every sync event; SwiftUI
/// rebuilt the bar every second or two during a backfill, dropping Postio's
/// menus each time, and `MenuBar` put them back each time: `File` blinking
/// in and out of the menu bar, logged as a restore every 1.5-3 s. Read here,
/// in a view's own body, a count changing redraws this window and nothing
/// else.
private struct SettingsWindow: View {
    let engine: Engine
    let settings: SettingsStore

    var body: some View {
        SettingsPaneView(
            store: settings,
            accounts: engine.accounts,
            mailboxes: engine.mailboxes,
            actions: engine.settingsActions,
            accountCursor: engine.settingsAccounts,
            repair: engine.accountRepair,
            reloadAccounts: { engine.refreshAccounts() },
            session: engine.session
        )
        .preferredColorScheme(engine.colorScheme)
        .background(WindowConfigurator(role: .settings))
    }
}

/// One compose window's content, for the same reason as `SettingsWindow`:
/// its reads -- the open drafts, the session, the theme -- belong to this
/// window, not to the scene graph the menu bar is built from.
private struct ComposeWindow: View {
    let engine: Engine
    let draft: Int64?

    var body: some View {
        if let draft, let model = engine.compose.model(draft), let session = engine.session {
            ComposeView(
                session: session,
                model: model,
                close: { engine.compose.close(draft) }
            )
            .preferredColorScheme(engine.colorScheme)
            .navigationTitle(model.title)
            .background(WindowConfigurator(role: .compose, draft: draft))
        }
    }
}
