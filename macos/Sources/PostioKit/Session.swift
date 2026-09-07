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

    /// The `[ui]` table this session was opened with.
    ///
    /// Row density, theme, and what a row draws. Read from here rather than
    /// from `config.toml` by whoever is drawing, so the list and the settings
    /// window cannot end up with two opinions of the same table.
    public func appearance() -> AppearanceFfi { inner.appearance() }

    /// The verbs the focused row announces, from this session's bindings.
    public func rowHints() -> [RowHintFfi] { inner.rowHints() }

    /// Every configured account, as the settings pane lists them.
    public func accounts() -> [AccountFfi] { inner.accounts() }

    /// Show `scope`, and answer the generation the window is now on.
    @discardableResult
    public func openScope(_ scope: ScopeFfi) -> UInt64 { inner.openScope(scope: scope) }

    /// Read a conversation into the reading pane.
    ///
    /// Returns at once. `UiEvent.conversationReady` says when there is
    /// something to draw and `conversation` is what to draw — the same
    /// local-first shape every other read here has: nothing awaits I/O.
    ///
    /// The list is untouched. A conversation is what the *pane* shows; the
    /// list stays the list, and there is no drill-in on either platform.
    public func openConversation(_ thread: Int64) { inner.openConversation(thread: thread) }

    /// The conversation the pane is showing, folded — `nil` until one has
    /// been asked for.
    ///
    /// Already stacked, already focused, already expanded to the cap. **None
    /// of that is Swift's to decide**: which messages open is what an open
    /// conversation costs, one web view each, and `postio_ui::conversation`
    /// answers it for both frontends.
    public var conversation: ConversationFfi? { inner.conversation() }

    /// Which runs of collapsed messages fold into one divider.
    ///
    /// Asked again on every expand and collapse, because that is when the
    /// answer changes. The three-in-a-row minimum and the eliding of the
    /// names are the boundary's, not this frontend's.
    /// Static because it is a function, not a question about this session:
    /// it reads no store and holds no state, and a conversation pane asks it
    /// on every keystroke that changes what is open.
    public static func runs(rows: [RowFfi], expanded: [Bool]) -> [RunFfi] {
        conversationRuns(rows: rows, expanded: expanded)
    }

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
    /// `original` is the one gesture that leaves reader view: what the
    /// sender wrote, on their own paper-white sheet, inset from Postio's
    /// chrome. Per message and per view — nothing is remembered, so the next
    /// message opens reduced again.
    public func readerDocument(
        message: Int64,
        remote: RemoteImagesFfi,
        original: Bool = false
    ) -> String {
        inner.readerDocument(message: message, remote: remote, original: original)
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

    /// Run `query`, and show its hits as the list.
    ///
    /// **One query language.** `postio-search` parses the operators, behind
    /// the boundary, for both frontends — Swift does not re-implement
    /// `from:` or `is:unread`, or the two platforms would accept different
    /// queries. Answers the generation the window is now on, the same as
    /// `openScope`, and the rows page in behind exactly as a folder's do:
    /// a search matching forty thousand messages is a count and a few
    /// resident pages.
    @discardableResult
    public func search(_ query: String) -> UInt64 { inner.search(query: query) }

    /// Leave search and restore the scope that was open.
    ///
    /// Restores rather than reloads: the boundary remembered what was on
    /// screen, so this costs nothing where re-opening the folder would cost a
    /// count and a page.
    @discardableResult
    public func clearSearch() -> UInt64 { inner.clearSearch() }

    /// What the last search turned out to be, or `nil` outside a search.
    ///
    /// The wording is `postio_ui::search::readout`'s, including its caveats —
    /// "still syncing" is a state that ends (#352), and an account named
    /// unreachable is ADR 0005 Q10's promise that a view says what it left
    /// out. Neither is worth a second frontend re-deriving.
    public var searchOutcome: OutcomeFfi? { inner.searchOutcome() }

    /// Whether the list is showing search results rather than a folder.
    public var isSearching: Bool { inner.isSearching() }

    /// The excerpt for `message`, with the match located.
    ///
    /// Text and byte ranges, never marked-up text — the same decision the
    /// palette's highlighting makes, and for the same reason: one answer
    /// about what matched, drawn each frontend's own way.
    public func snippet(for message: Int64) -> SnippetFfi? {
        inner.snippetFor(message: message)
    }

    /// Whether `id` can run in `context`, given the open view.
    ///
    /// What a menu asks before drawing an item enabled. The same question the
    /// palette's filter answers, so the two cannot disagree — before #1158
    /// only the palette was asking, and the menu drew Send and the whole
    /// Format menu as live options against a build with no composer.
    public func isAvailable(_ id: String, in context: UiContext) -> Bool {
        inner.isAvailable(id: id, context: context)
    }

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

    /// The cursor rested on `message` long enough for it to count as read.
    ///
    /// Not `invoke`: `MarkReadOnDwell` is deliberately outside the registry,
    /// because it is the one dispatch that is *not* recorded on the undo
    /// stack — `u` takes back what you did, and reading a mailbox produces
    /// one of these per message rested on.
    public func markReadOnDwell(_ message: Int64) {
        inner.markReadOnDwell(message: message)
    }

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

    /// The chord a surface draws beside a command, or `nil` when there is
    /// none to draw.
    ///
    /// **The one place a key is turned into glyphs.** Both keyboard layers
    /// are live, and every surface that names a key wants the same one — the
    /// `⌘` chord, falling back to the mnemonic — so asking here is what stops
    /// one button saying `⌘R` and the one beside it saying `E`. It did:
    /// the conversation pane drew mnemonics for a week because it asked for
    /// the *primary* binding, which is the other layer.
    public func accelerator(for command: String) -> String? {
        MenuPlan.accelerator(among: bindings(for: command))
    }

    /// Every binding in force for a command, the primary first.
    ///
    /// Both of the canvas' keyboard layers, because they are two bindings on
    /// one command rather than a mode: `e` replies and so does `⌘R`. A menu
    /// draws the chord, the cheat sheet lists both, and neither decides which
    /// exists.
    public func bindings(for command: String) -> [String] {
        inner.bindingsFor(command: command)
    }

    /// What the reader is holding back for `message`, or `nil` when nothing
    /// is — a notice with nothing to report teaches people to dismiss the
    /// one that matters.
    /// Who `message` was addressed to, already rendered.
    ///
    /// `nil` for a message the store does not hold. A read of its own rather
    /// than a field on the row: the list draws no recipients, and paying for
    /// them per row would load a mailbox's addresses to show one message's.
    /// The composer's editing bridge — the one script Postio runs.
    ///
    /// `postio_ui::compose::EDITOR_SCRIPT`, the same bytes WebKitGTK
    /// injects. It crosses rather than being written again here because it
    /// decides the *dialect* the surface emits: a second copy would produce
    /// `<div>`s where this one produces `<p>`s, the boundary would narrow
    /// them differently, and the two composers would disagree about what the
    /// same keystrokes wrote while both still round-tripped cleanly.
    public func editorScript() -> String { inner.editorScript() }

    /// The script that applies a mark to the composer's selection, or `nil`
    /// for a command that is not one of the marks.
    public func markScript(_ command: String) -> String? {
        inner.markScript(command: command)
    }

    /// The script that links the selection to `href`, or `nil` when a
    /// message may not point there.
    public func linkScript(_ href: String) -> String? {
        inner.linkScript(href: href)
    }

    /// Narrow pasted markup to what a message may carry, and say what that
    /// cost.
    ///
    /// Pure: no store, no network. The sentence in `dropped` is the
    /// engine's, so both composers say the same thing about the same paste.
    public func narrowPaste(_ html: String) -> PastedFfi {
        inner.narrowPaste(html: html)
    }

    /// The draft `id` as the store has it, or `nil`.
    public func draft(_ id: Int64) -> DraftFfi? { inner.draft(id: id) }

    public func recipients(_ message: Int64) -> RecipientsFfi? {
        inner.recipients(message: message)
    }

    /// The verbs the reading pane offers, in canvas order.
    ///
    /// No key comes with them — `accelerator(for:)` is what spells one on
    /// this platform. What crosses is which verbs and in what order, which is
    /// the same on both frontends by construction.
    public func readerActions() -> [ReaderActionFfi] { inner.readerActions() }

    public func readerNotice(_ message: Int64) -> ReaderNoticeFfi? {
        inner.readerNotice(message: message)
    }

    /// Always allow this address's remote images, across restarts.
    ///
    /// POSTIO-CONSENT: only ever from the popover's own item, which names
    /// the address it is about.
    public func allowSender(_ address: String) { inner.allowSender(address: address) }

    /// Always allow every address at this domain.
    public func allowDomain(_ domain: String) { inner.allowDomain(domain: domain) }

    /// Add an account that signs in with a password.
    ///
    /// `nil` when it was added; a sentence when it was not. The password goes
    /// to the OS keyring under the address and nowhere else — never
    /// `config.toml`, never a log (ADR 0014).
    public func addImapAccount(
        address: String,
        password: String,
        imapHost: String,
        imapPort: UInt16,
        smtpHost: String,
        smtpPort: UInt16
    ) -> String? {
        inner.addImapAccount(
            address: address,
            password: password,
            imapHost: imapHost,
            imapPort: imapPort,
            smtpHost: smtpHost,
            smtpPort: smtpPort
        )
    }

    /// Open a session against this account's server and close it again.
    ///
    /// **Blocks**, so it belongs on a detached task. It is the same path sync
    /// takes — same credential, same settings — which is what makes a test
    /// that passes a statement about sync rather than about the button.
    public func testConnection(_ account: Int64) -> ConnectionReportFfi {
        inner.testConnection(account: account)
    }

    /// Change what an account calls itself — the one field the account form
    /// edits. Everything else on a row came from the preset table or from a
    /// sign-in, and editing those is changing which account this is.
    public func setDisplayName(_ account: Int64, to name: String) -> String? {
        inner.setDisplayName(account: account, name: name)
    }

    /// How much disk this account's mail takes, in words — or `nil` when
    /// there is nothing to weigh. A fresh account says nothing rather than
    /// `0 B`, which reads as a failure.
    public func weight(of account: Int64) -> String? {
        inner.accountWeight(account: account)
    }

    /// Rebuild this account's search index from the mail already here.
    /// Blocks, and reaches no server.
    public func reindexAccount(_ account: Int64) -> String? {
        inner.reindexAccount(account: account)
    }

    /// Take an account away — its row, and its credentials. A mail client
    /// that forgets an account and keeps its password is worse than one that
    /// does not forget it.
    public func removeAccount(_ account: Int64) -> String? {
        inner.removeAccount(account: account)
    }

    /// Sign in to `address` through the system browser and add the account.
    ///
    /// **Blocks until the flow is over** — it is waiting on a person in
    /// another application — so it belongs on a detached task, the way
    /// opening a session does. `nil` when the account was added; a sentence
    /// otherwise, including when the user closed the tab.
    ///
    /// The client id is the user's own. Postio ships none (ADR 0006 Q1): a
    /// credential inside an open-source application is one every user of it
    /// shares.
    public func signInWithBrowser(
        address: String,
        clientId: String,
        clientSecret: String?
    ) -> String? {
        inner.signInWithBrowser(
            address: address, clientId: clientId, clientSecret: clientSecret)
    }

    /// What the sign-in in flight is doing — the loopback port, mostly.
    public var signInProgress: SignInProgressFfi { inner.signInProgress() }

    /// Give up on the sign-in in flight. Closing the sheet means this.
    public func cancelSignIn() { inner.cancelSignIn() }

    // -- writing mail (#1272) ---------------------------------------------

    /// A new message, from the account that would send it.
    ///
    /// `nil` when no account is configured, which is a real state on a fresh
    /// install rather than an error: there is nothing to send from yet.
    public func newDraft() -> DraftFfi? { inner.newDraft() }

    /// A reply to `message` — to its sender, or to everyone on it.
    ///
    /// Who that is, what the subject becomes and what the quote looks like
    /// are `postio_model::reply`'s answers, shared with the frontend that
    /// already had them. Swift addresses nothing itself.
    public func replyDraft(to message: Int64, all: Bool) -> DraftFfi? {
        inner.replyDraft(message: message, all: all)
    }

    /// A forward of `message`, addressed to nobody yet.
    public func forwardDraft(_ message: Int64) -> DraftFfi? {
        inner.forwardDraft(message: message)
    }

    /// Attach a file to a draft, and answer the draft with it on.
    ///
    /// The MIME type is sniffed here because that is a platform service:
    /// macOS asks `UniformTypeIdentifiers`, freedesktop reads
    /// shared-mime-info, and neither can answer for the other. Everything
    /// else — the size guard, the blob write, the row — happens once, on the
    /// other side of this call.
    public func attach(_ file: URL, to draft: DraftFfi) throws -> DraftFfi {
        try inner.attachToDraft(
            draft: draft,
            path: file.path,
            mimeType: MimeType.of(file)
        )
    }

    /// Write the draft where another editor can open it, and answer where.
    ///
    /// The file is the user's alone — a private directory, mode 0600 — for
    /// the reason the shared code records: a draft is mail that has not been
    /// sent, which is often the most private mail there is.
    public func beginHandoff(of draft: DraftFfi) throws -> String {
        try inner.beginHandoff(draft: draft)
    }

    /// Take back what the other editor wrote.
    public func endHandoff(of draft: DraftFfi, at path: String) throws -> DraftFfi {
        try inner.endHandoff(draft: draft, path: path)
    }

    /// Take an attachment off a draft.
    public func detach(_ attachment: Int64, from draft: DraftFfi) throws -> DraftFfi {
        try inner.detachFromDraft(draft: draft, attachment: attachment)
    }

    /// Write the draft to the store; answers it with the id it now has.
    public func saveDraft(_ draft: DraftFfi) -> DraftFfi? {
        inner.saveDraft(draft: draft)
    }

    /// Queue the draft for sending. `nil` when it went; a sentence when it
    /// could not, which the composer shows rather than closing over.
    public func sendDraft(_ draft: DraftFfi) -> String? {
        inner.sendDraft(draft: draft)
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
