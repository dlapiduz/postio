import PostioKit
import SwiftUI

/// The engine, and what to show when it will not start.
///
/// Opening a session reads the local store's key from the login Keychain
/// (ADR 0014), and an ad-hoc-signed build has a new code identity on every
/// rebuild — so macOS asks again after each one. That is a real state the
/// application has to render rather than crash through, and it is the reason
/// this holds a *result* rather than a session.
@Observable
final class Engine {
    enum State {
        /// A live session, with the list driven from it.
        case open(MessageTableController)
        /// No session, and the sentence explaining why.
        case unavailable(String)
    }

    private(set) var state: State
    private var session: PostioSession?

    init() {
        do {
            let session = try PostioSession.open()
            self.session = session
            state = .open(MessageTableController(source: SessionRowSource(session: session)))
        } catch {
            // The message the boundary wrote, not one invented here: a locked
            // keychain says how to unlock it, and a broken store says what
            // broke. Replacing either with "could not start" throws away the
            // only instruction the user gets.
            state = .unavailable(String(describing: error))
            session = nil
        }
    }

    /// How many rows the open scope has, or zero when there is no session.
    var rowCount: UInt32 { session?.rowCount ?? 0 }

    deinit { session?.shutdown() }
}
