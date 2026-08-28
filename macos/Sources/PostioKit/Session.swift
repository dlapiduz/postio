import Foundation
import PostioFFI

/// Postio's engine, as the macOS application holds it.
///
/// Everything below this is Rust: the store, the sync engine, the protocol
/// crates, the command registry and the reader's document assembly. This type
/// exists so the rest of the application talks to *it* rather than to the
/// generated bindings directly — the bindings are regenerated on every build
/// and their shape follows the Rust, so a view that imported them would be
/// coupled to a file nobody edits.
///
/// It deliberately adds no behaviour of its own. Anything that looks like a
/// decision — which command a key runs, what a reader document contains, how
/// a list pages — belongs on the other side of the boundary, where both
/// frontends share it (ADR 0019).
public final class PostioSession {
    private let inner: Session

    /// Opens a session over the store at the platform's usual path.
    ///
    /// Blocks: the store's key comes from the OS keyring and that round trip
    /// can wait on a user prompt, so this belongs in a launch task and never
    /// on the main actor. A locked keyring arrives as `.keyringLocked`, which
    /// is a different surface from a broken store — "unlock and retry", not
    /// "set up an account you already have".
    public static func open() throws -> PostioSession {
        PostioSession(inner: try Session.openAt(storePath: nil))
    }

    private init(inner: Session) {
        self.inner = inner
    }

    /// Whether the session still holds its store.
    public var isOpen: Bool { inner.isOpen() }

    /// Every command the registry knows, in cheat-sheet order.
    ///
    /// The palette, the cheat sheet and the menu bar are all built from this
    /// rather than from a list kept in Swift, so a command added in Rust
    /// reaches this application without anybody editing it.
    public var commands: [CommandSpecFfi] { inner.commands() }

    /// Show `scope`, and answer the generation the window is now on.
    @discardableResult
    public func openScope(_ scope: ScopeFfi) -> UInt64 { inner.openScope(scope: scope) }

    /// How many rows the current scope has.
    ///
    /// A `COUNT` on the other side, not the length of anything: a hundred
    /// thousand rows are a number here, never a hundred thousand structs.
    public var rowCount: UInt32 { inner.rowCount() }

    /// The row at `position`, or `nil` while its page is on its way.
    ///
    /// Synchronous and does no I/O. A `nil` means draw a placeholder — the
    /// fetch is already running by the time this returns, and
    /// `UiEvent.pageReady` says when to ask again.
    public func row(at position: UInt32) -> RowFfi? { inner.rowAt(position: position) }

    /// Drops the store and ends the event drain.
    public func shutdown() { inner.shutdown() }

    /// The next event, or `nil` once the session has stopped.
    ///
    /// Driven as `while let event = await session.nextEvent()` on the main
    /// actor: the same drain the GTK window runs on its main context, so no
    /// backend work reaches the UI thread on either platform.
    public func nextEvent() async -> UiEvent? { await inner.nextEvent() }
}
