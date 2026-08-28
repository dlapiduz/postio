import PostioKit
import SwiftUI

/// The application.
///
/// Useless on purpose, for now. It opens a session and shows what came back
/// through the boundary — which is the only thing worth asserting at this
/// stage, because every other part of the chain (cargo, the bindings
/// generator, the module map, the linker, the bundle) fails in its own way and
/// none of them is covered by anything else.
@main
struct PostioApp: App {
    var body: some Scene {
        WindowGroup("Postio") {
            BoundaryProof()
        }
        .defaultSize(width: 520, height: 320)
    }
}

/// What the engine answered, drawn plainly.
struct BoundaryProof: View {
    // `Outcome`, not `State`: a nested `enum State` shadows SwiftUI's
    // `@State` property wrapper, and the error it produces names neither.
    @State private var outcome: Outcome = .opening

    enum Outcome {
        case opening
        case open(commands: Int, first: String)
        case failed(String)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Postio").font(.largeTitle)
            switch outcome {
            case .opening:
                Text("Opening a session…").foregroundStyle(.secondary)
            case let .open(commands, first):
                // A number Swift could not have known on its own. "A window
                // appeared" would pass with the boundary entirely broken.
                Text("The engine answered.").font(.headline)
                Text("\(commands) commands in the registry")
                Text("first: \(first)").foregroundStyle(.secondary).monospaced()
            case let .failed(why):
                Text("The engine did not open.").font(.headline)
                Text(why).foregroundStyle(.red)
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .task { outcome = Self.open() }
    }

    static func open() -> Outcome {
        do {
            let session = try PostioSession.open()
            defer { session.shutdown() }
            let commands = session.commands
            return .open(commands: commands.count, first: commands.first?.id ?? "none")
        } catch {
            return .failed(String(describing: error))
        }
    }
}
