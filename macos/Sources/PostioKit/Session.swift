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

    /// Turn the log on, before anything can have anything to say.
    ///
    /// `POSTIO_LOG` and `[logging]` in `config.toml`, the same two controls
    /// the GTK build has. Worth calling first rather than at leisure: opening
    /// a session reads the Keychain and migrates the store, and both can fail
    /// before there is any UI to report it in — which on this platform used to
    /// mean a blank window and no way to ask why.
    public static func startLogging() { PostioFFI.startLogging() }

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

    /// What one key press means here.
    ///
    /// **The whole of this application's keyboard, and it decides nothing.**
    /// `KeyEvent.reduce` turns an `NSEvent` into the three things every
    /// toolkit can supply and this asks; `postio_ui::keymap` owns the table,
    /// the chords, the sequences and the leader timeout, for both frontends
    /// (ADR 0019 Q4). There is deliberately no Swift keymap to disagree with
    /// it, and no `.keyboardShortcut`, which could express none of the three.
    ///
    /// `typing` is whether the focused surface takes text, and only the caller
    /// can see that. It is the difference between a search field that takes
    /// `a` and a list that archives on it.
    public func key(
        _ reduced: KeyEvent.Reduced,
        in context: UiContext,
        typing: Bool
    ) -> KeyOutcomeFfi {
        inner.key(
            character: reduced.character,
            name: reduced.name,
            modifiers: reduced.modifiers,
            context: context,
            inTextEntry: typing
        )
    }

    /// Run a command, aimed the way the current view says it should be.
    ///
    /// Nothing comes back, and that is the architecture rather than an
    /// omission: a verb writes to SQLite, enqueues and returns, and what
    /// happened arrives on `nextEvent`. The UI never awaits the network.
    public func invoke(_ id: String) { inner.invoke(id: id) }

    /// Report where the keyboard is, so a verb with nothing marked knows
    /// which row it is about.
    ///
    /// The *cursor*, not the selection: `docs/PRODUCT.md` §9 keeps them
    /// separate, and moving down the list must not build a selection.
    public func setCursor(_ message: Int64?) { inner.setCursor(message: message) }

    /// The palette's rows for `query`, best first.
    ///
    /// Already ranked and already filtered to what `context` can run.
    /// **Do not sort or filter these again**: the ranking is
    /// `postio_ui::palette`'s, and a second one means the same query offers
    /// different things on each platform.
    public func paletteEntries(_ query: String, in context: UiContext) -> [PaletteEntryFfi] {
        inner.paletteEntries(query: query, context: context)
    }

    /// Every command reachable in `context`, with the binding in force.
    ///
    /// The same list the palette reads, unfiltered — one list read two ways.
    public func cheatSheet(in context: UiContext) -> [PaletteEntryFfi] {
        inner.cheatSheet(context: context)
    }

    /// Whether `message` is *marked*.
    ///
    /// Not whether it is under the cursor: `PRODUCT.md` §9 keeps those apart,
    /// and `NSTableView`'s own selection is the cursor here. Answered without
    /// enumerating a whole-view selection, which is what makes "select all,
    /// then deselect three" cost three ids rather than a hundred thousand.
    public func isSelected(_ message: Int64) -> Bool { inner.isSelected(message: message) }

    /// What to show above the list — "12 selected" — or nothing.
    ///
    /// From the model, which knows the answer for a whole-view selection
    /// without listing it. Counting ids on this side could not draw the one
    /// case that most needs a count.
    public var selectionSummary: String? { inner.selectionSummary() }

    /// The row the cursor is on, or `nil` when the list has none.
    public var cursorRow: UInt32? { inner.cursorRow() }

    /// The message the cursor is on, if its page has arrived.
    public var cursorMessage: Int64? { inner.cursorMessage() }

    /// Put the cursor on `row` — what a click on the list means.
    ///
    /// The position, not just the message: after a click, `j` has to move
    /// from where the user clicked.
    public func setCursorRow(_ row: UInt32?) { inner.setCursorRow(row: row) }

    /// Mark `message`, or take it out of the selection again.
    public func toggleSelection(_ message: Int64) { inner.toggleSelection(message: message) }

    /// Unmark everything.
    public func clearSelection() { inner.clearSelection() }

    /// The binding in force for a command, for drawing an accelerator.
    ///
    /// The user's override if there is one, the built-in default otherwise,
    /// and resolved for this platform — so a Mac gets `cmd+k` rather than the
    /// `mod+k` the table stores. A menu that read `defaultBinding` directly
    /// would show the wrong key for a rebound command, which is worse than
    /// showing none.
    public func binding(for command: String) -> String? {
        inner.bindingFor(command: command)
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
