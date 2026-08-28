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
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup("Postio") {
            Shell(engine: engine)
                .background(WindowConfigurator())
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
    }
}
