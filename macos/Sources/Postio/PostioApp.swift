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

    var body: some Scene {
        WindowGroup("Postio") {
            Shell(engine: engine)
        }
        .defaultSize(width: 1100, height: 700)
        .windowToolbarStyle(.unified)
    }
}
