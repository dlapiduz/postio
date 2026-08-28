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
    var body: some Scene {
        WindowGroup("Postio") {
            BoundaryProof()
        }
        .defaultSize(width: 560, height: 380)
    }
}

/// What the engine answered, drawn plainly.
///
/// Reads the registry rather than opening a session. A session would read the
/// store's key from the login Keychain, and an unsigned build's identity
/// changes on every rebuild — so launching would prompt, before there is any
/// mail to show.
struct BoundaryProof: View {
    private let commands = PostioRegistry.commands

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Postio").font(.largeTitle.weight(.semibold))
                // A number Swift could not have known on its own. "A window
                // appeared" would pass with the boundary entirely broken.
                Text("\(commands.count) commands, straight from the Rust registry")
                    .foregroundStyle(.secondary)
            }

            Divider()

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 6) {
                    ForEach(commands, id: \.id) { command in
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            Text(command.defaultBinding)
                                .monospaced()
                                .frame(width: 90, alignment: .leading)
                                .foregroundStyle(.secondary)
                            Text(command.title)
                        }
                    }
                }
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
