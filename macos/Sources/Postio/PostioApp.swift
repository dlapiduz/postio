import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI

@main
struct PostioApp: App {
    @State private var engine = Engine()
    @State private var settings = SettingsStore(path: (try? settingsPath()) ?? "")
    @Environment(\.scenePhase) private var phase
    // `onOpenURL` only fires for an existing scene, and a `mailto:` click is how Postio is often
    // launched.
    @NSApplicationDelegateAdaptor(URLHandler.self) private var urls

    var body: some Scene {
        WindowGroup("Postio") {
            MainWindow(engine: engine)
                .background(WindowConfigurator())
                .preferredColorScheme(engine.colorScheme)
                // The delegate exists before a session, so it gets a closure: a `mailto:` during
                // launch finds what is true when clicked.
                .onAppear {
                    urls.write = { engine.write(mailto: $0) }
                    urls.follow = { engine.follow($0) }
                    urls.stop = { engine.shutdown() }
                }
        }
        .onChange(of: phase) { _, now in
            // Not on `.background`: `⌘W`, `⌘H` and minimising all produce it, and ending the
            // session there stopped every sync and IDLE connection with no way back (the session
            // opens once). Shutdown lives in `applicationWillTerminate`; `SessionLifetime` is the
            // rule.
            if SessionLifetime.shouldEnd(on: SessionPhase(now)) { engine.shutdown() }
        }
        .defaultSize(width: 1440, height: 900)
        .windowToolbarStyle(.unified)
        // Size and position use `NSWindow`'s own autosave, the only thing that survives a window
        // being closed and reopened.
        .windowResizability(.contentMinSize)

        // `Window`, not the `Settings` scene: the scene opens only by a selector that reached no
        // handler here (#1261), while `openWindow` is a call.
        Window("Settings", id: WindowId.settings) {
            SettingsWindow(engine: engine, settings: settings)
        }
        .defaultSize(width: 900, height: 560)
        .windowResizability(.contentSize)

    }
}

/// `terminating` has no `ScenePhase`; it arrives from `applicationWillTerminate`.
extension SessionPhase {
    init(_ phase: ScenePhase) {
        switch phase {
        case .active: self = .active
        case .inactive: self = .inactive
        case .background: self = .background
        @unknown default: self = .inactive
        }
    }
}

/// The Settings window's content, as a view of its own: inline in `PostioApp.body` it read
/// `engine.mailboxes`, which moves every sync event, rebuilding the menus each time (`File` blinked
/// in the menu bar). Read in a view's own body, a change redraws only this window.
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
            session: engine.session,
            contacts: engine.contactsSettings,
            section: engine.settingsSection,
            run: engine.session == nil ? nil : { command in engine.runFromSettings(command) }
        )
        .preferredColorScheme(engine.colorScheme)
        .background(WindowConfigurator(role: .settings))
        // `nil` until it lands, so the pane says nothing rather than a zero.
        .task {
            guard let session = engine.session else { return }
            let today = await Task.detached { try? session.focusCounts().filteredToday }.value
            settings.filteredToday = today
        }
    }
}
