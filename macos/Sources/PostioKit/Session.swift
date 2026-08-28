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
/// Safe to hand across threads.
///
/// The Rust type behind it is `Send + Sync` — uniffi requires that of an
/// exported object, and every field of ours is behind a lock or an `Arc`. That
/// is what lets a reader document be built off the main actor instead of on
/// the cursor's own thread.
extension PostioSession: @unchecked Sendable {}

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

    /// Every folder of every enabled account.
    ///
    /// A flat list carrying parent ids: the tree is rebuilt for display rather
    /// than crossing as nested structures, which keeps the boundary's types
    /// simple and loses nothing — the nesting is in `parent`.
    public var mailboxes: [MailboxFfi] { inner.mailboxes() }

    /// Show `scope`, and answer the generation the window is now on.
    @discardableResult
    public func openScope(_ scope: ScopeFfi) -> UInt64 { inner.openScope(scope: scope) }

    /// How many rows the current scope has.
    ///
    /// A `COUNT` on the other side, not the length of anything: a hundred
    /// thousand rows are a number here, never a hundred thousand structs.
    public var rowCount: UInt32 { inner.rowCount() }

    /// Run `query`, and answer the generation the list is now on.
    ///
    /// Swift parses none of it. `is:unread`, `from:`, a date range — the query
    /// language is `postio-search`'s on both platforms, because a second parser
    /// wearing the same syntax would accept different queries and nobody would
    /// notice until one of them refused something the other took.
    ///
    /// Synchronous: the index is local and the budget is under 100 ms. This is
    /// the one shape of engine call the UI is allowed to wait on, and only
    /// because it never touches the network.
    @discardableResult
    public func search(_ query: String) -> UInt64 { inner.search(query: query) }

    /// Put the folder that was open back in the list.
    ///
    /// The engine remembers which one, so clearing costs a count rather than a
    /// reload of the world — and the frontend does not hold navigation state
    /// that the GTK side would then hold differently.
    @discardableResult
    public func clearSearch() -> UInt64 { inner.clearSearch() }

    /// A hit's excerpt and the ranges within it that matched.
    ///
    /// `nil` for a row that is not a search hit, and for a hit whose body is
    /// not on this machine — no excerpt rather than a wrong one.
    public func snippet(for message: Int64) -> SnippetFfi? {
        inner.searchSnippet(message: message)
    }

    /// The row at `position`, or `nil` while its page is on its way.
    ///
    /// Synchronous and does no I/O. A `nil` means draw a placeholder — the
    /// fetch is already running by the time this returns, and
    /// `UiEvent.pageReady` says when to ask again.
    public func row(at position: UInt32) -> RowFfi? { inner.rowAt(position: position) }

    /// Tell the engine whether the machine currently has a connection.
    ///
    /// Reachability is a platform question, asked in the platform's own
    /// language (`Reachability`) and pushed down. Coming back from offline
    /// nudges a reconnect on the other side, so this is safe to call with the
    /// same answer repeatedly — `NWPathMonitor` does exactly that.
    public func setOffline(_ offline: Bool) { inner.setOffline(offline: offline) }

    /// Whether the platform has told the engine there is no connection.
    public var isOffline: Bool { inner.isOffline() }

    /// The whole document for a message, ready to hand a web view.
    ///
    /// Not fragments to assemble: the content security policy, the embedded
    /// font faces, the sanitized body inside its container and the scroll
    /// markers all come from the engine, which is what the GTK reader renders
    /// too. Swift composes no reader HTML.
    public func readerDocument(message: Int64, remote: RemoteImagesFfi) -> String {
        inner.readerDocument(message: message, remote: remote)
    }

    /// One inline part of `message`, by its `Content-ID`.
    ///
    /// `nil` when the bytes are not already on this machine — the privacy
    /// commitment rather than a gap to fill in later. Fetching here would be
    /// the tracking pixel the reader blocks, arriving through the back door.
    public func resolveCid(message: Int64, contentId: String) -> InlinePart? {
        inner.resolveCid(message: message, contentId: contentId)
    }

    /// Start syncing every configured account; answers how many started.
    @discardableResult
    public func startSyncing() throws -> UInt32 { try inner.startSyncing() }

    /// How many accounts are configured and enabled.
    public var configuredAccounts: UInt32 { inner.configuredAccounts() }

    /// Drops the store and ends the event drain.
    public func shutdown() { inner.shutdown() }

    /// The next event, or `nil` once the session has stopped.
    ///
    /// Driven as `while let event = await session.nextEvent()` on the main
    /// actor: the same drain the GTK window runs on its main context, so no
    /// backend work reaches the UI thread on either platform.
    public func nextEvent() async -> UiEvent? { await inner.nextEvent() }
}
