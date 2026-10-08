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

    /// Say what sync would have said about a demo's account
    /// (`DemoMode.state`); `false` for an unknown word, and always outside a
    /// demo build.
    public func demoState(_ state: String) -> Bool { inner.demoState(state: state) }

    /// Opens a session over the demo store `seed` names, in memory
    /// (`DemoMode`). Reads no Keychain; refused by a build without demos.
    public static func openDemo(_ seed: String) throws -> PostioSession {
        PostioSession(inner: try Session.openDemo(seed: seed))
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

    /// The key hints the search bar announces — `Ret open · Tab refine ·
    /// C-s save as folder`. From this session's keymap, so a rebinding
    /// reaches the footer.
    public func searchHints() -> [KeyHintFfi] { inner.searchHints() }
    /// Every configured account, as the settings pane lists them.
    public func accounts() -> [AccountFfi] { inner.accounts() }

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

    // MARK: Focus's list

    /// Show one of Focus's lists. Counted first, then `FocusListChanged`
    /// says how long it is; pages follow as `FocusPageReady`.
    public func openFocus(_ scope: FocusScopeFfi) { inner.openFocus(scope: scope) }

    /// A plain click put the cursor on `position`. The controller answers
    /// with `FocusCursor` on `nextEvent`; nothing moves before that.
    public func focusPoint(_ position: Int) {
        guard let row = UInt32(exactly: position) else { return }
        inner.focusPoint(position: row)
    }

    /// A modified click on `position`: `range` for ⇧ (from the anchor), a
    /// toggle for ⌘. Answered with `FocusSelection` and `FocusCursor`.
    public func focusPick(_ position: Int, range: Bool) {
        guard let row = UInt32(exactly: position) else { return }
        inner.focusPick(position: row, range: range)
    }

    /// Whether the list stands scrolled to its very top: where an undo
    /// that brings rows in above leaves it.
    public func focusAtTop(_ atTop: Bool) { inner.focusAtTop(atTop: atTop) }

    /// What Undo would take back now, in the toast's words, or `nil`. A
    /// store read: off the main actor (`PostioUndoManager.refresh`).
    public nonisolated func undoDescription() -> String? { inner.undoDescription() }

    /// The header strip's counts, read now.
    public func focusCounts() throws -> FocusCountsFfi { try inner.focusCounts() }

    /// The header strip's words, composed by the engine (`focus_strip`).
    public func focusStrip() throws -> FocusStripFfi { try inner.focusStrip() }

    // MARK: Focus's surfaces

    /// A surface opened over the list: the keys are its now (the
    /// controller's Reader context for the message window), and on the
    /// Mac it replaces the secondary window that was open (M4).
    public func focusSurfaceOpened(_ kind: SurfaceKindFfi) { inner.focusSurfaceOpened(kind: kind) }

    /// A surface over the list closed, however it closed: ⌘W, its close
    /// button, or the controller's own `FocusCloseSurface`.
    public func focusSurfaceClosed(_ kind: SurfaceKindFfi) { inner.focusSurfaceClosed(kind: kind) }

    // MARK: the composer (T079)

    /// Something was written in the composer: said on every edit.
    public func focusComposerEdited() { inner.focusComposerEdited() }

    /// How the save `FocusSaveDraft { composition }` asked for went.
    public func focusDraftSaved(_ composition: UInt64, kept: Bool, error: String?) {
        inner.focusDraftSaved(composition: composition, kept: kept, error: error)
    }

    /// What completes `text` in a recipient field of `account`'s draft:
    /// mail's correspondents and groups, with what Contacts lent ranked
    /// among them (T075). Blocks on the store: off the main actor.
    public nonisolated func recipientSuggestions(
        account: Int64, text: String, limit: UInt32, extra: [ExternalContactFfi]
    ) -> [RecipientSuggestionFfi] {
        inner.recipientSuggestions(account: account, text: text, limit: limit, extra: extra)
    }

        // MARK: the command bar and the folders popover (T084-T086)

    /// The bar's field holds `text` now: said on every change.
    public func focusBarTyped(_ text: String) { inner.focusBarTyped(text: text) }

    /// Run the bar's line `token`: Return on the highlighted line, or a click.
    public func focusBarRun(_ token: UInt64) { inner.focusBarRun(token: token) }

    /// `Tab` in the bar's field; `false` leaves the key to the toolkit.
    public func focusBarTab() -> Bool { inner.focusBarTab() }

    /// The folders popover's places whose names hold `filter`, read now
    /// from the last read (ask again on `FocusPlacesChanged`).
    public func focusPlaces(_ filter: String) -> [PlaceEntryFfi] { inner.focusPlaces(filter: filter) }

    /// Go to the popover's place `token`.
    public func focusOpenPlace(_ token: UInt64) { inner.focusOpenPlace(token: token) }

    /// What the popover's filter says before anything is typed.
    public func focusPlacesPlaceholder() -> String { inner.focusPlacesPlaceholder() }

    // MARK: the pickers at the row (T091, T092)

    /// The picker's field holds `text` now: said on every change.
    public func focusPickerTyped(_ text: String) { inner.focusPickerTyped(text: text) }

    /// The picker's row `token` chosen: a click, or Return on the highlight.
    /// A preset or a folder acts and closes; a label goes on or off.
    public func focusPickerChoose(_ token: UInt64) { inner.focusPickerChoose(token: token) }

    /// Space on the picker's highlighted row `token`: a label on or off.
    public func focusPickerToggle(_ token: UInt64) { inner.focusPickerToggle(token: token) }

    // MARK: Filtered, the digest, the rule sheet and capture (T113-T117)

    /// Filtered's row `index` was clicked: the keyboard goes there.
    public func focusFilteredPoint(_ index: UInt32) { inner.focusFilteredPoint(index: index) }

    /// Filtered was scrolled to the end of the rows it has: read more.
    public func focusFilteredMore() { inner.focusFilteredMore() }

    /// The digest list's row `index` was clicked.
    public func focusDigestPoint(_ index: UInt32) { inner.focusDigestPoint(index: index) }

    /// The digest summary's reference `index` (in reading order) was clicked.
    public func focusDigestReference(_ index: UInt32) { inner.focusDigestReference(index: index) }

    /// Yes to the `FocusConfirm` named `token`.
    public func focusConfirmed(_ token: UInt64) { inner.focusConfirmed(token: token) }

    /// The rule sheet's query field holds `text` now.
    public func focusRuleQuery(_ text: String) { inner.focusRuleQuery(text: text) }

    /// "Match a list or a search instead…".
    public func focusRuleMatchInstead() { inner.focusRuleMatchInstead() }

    /// "Digest mail like this".
    public func focusRuleLikeThis() { inner.focusRuleLikeThis() }

    /// The rule sheet's schedule, as its controls hold it now.
    public func focusRuleSchedule(_ schedule: RuleScheduleFfi) { inner.focusRuleSchedule(schedule: schedule) }

    /// Create (or Save): write the rule.
    public func focusRuleCreate() { inner.focusRuleCreate() }

    /// Capture's text field holds `text` now.
    public func focusCaptureTyped(_ text: String) { inner.focusCaptureTyped(text: text) }

    /// Capture's due day, "YYYY-MM-DD", or `nil` for none.
    public func focusCaptureDue(_ day: String?) { inner.focusCaptureDue(day: day) }

    /// Capture's project filter holds `text` now.
    public func focusCaptureFilter(_ text: String) { inner.focusCaptureFilter(text: text) }

    /// Capture's project `token` was chosen.
    public func focusCaptureProject(_ token: UInt64) { inner.focusCaptureProject(token: token) }

    /// Open the message a `postio://` link names, or say why not.
    public func focusOpenLink(uri: String) { inner.focusOpenLink(uri: uri) }

    /// The open message's More menu and find, as they are now: what Back
    /// closes first.
    public func focusReaderState(moreOpen: Bool, finding: Bool) {
        inner.focusReaderState(moreOpen: moreOpen, finding: finding)
    }

    /// Always draw `sender`'s mail in `treatment`, or forget the choice
    /// with `nil`: "Always for this sender", in the file GTK keeps it in.
    public func alwaysTreatment(sender: String, treatment: TreatmentFfi?) {
        inner.alwaysTreatment(sender: sender, treatment: treatment)
    }

    /// What the message window draws around `message`'s body, shown from
    /// row `index` of `total`: composed in Rust (`focus_message_view`). A
    /// store read; never on the main actor.
    public nonisolated func focusMessageView(
        message: Int64, index: UInt32, total: UInt32
    ) -> FocusMessageViewFfi {
        inner.focusMessageView(message: message, index: index, total: total)
    }

    /// `message`'s treated body, in `chosen` when `O` switched it, and the
    /// message window's geometry beside a main window `mainWidth` wide
    /// (M1). A store read; never on the main actor.
    public nonisolated func focusReaderDocument(
        message: Int64, remote: RemoteImagesFfi, chosen: TreatmentFfi?, mainWidth: Int32
    ) -> FocusReaderDocumentFfi {
        inner.focusReaderDocument(
            message: message, remote: remote, chosen: chosen, mainWidth: mainWidth)
    }

    /// `message`'s raw source, for `v`. May fetch it from the server, the
    /// person having asked for these bytes by name: never on the main
    /// actor.
    public nonisolated func rawSource(_ message: Int64) throws -> Data {
        try inner.rawSource(message: message)
    }

    /// One of the reader's vendored faces by name, for `postio-font:`.
    public nonisolated func readerFont(_ name: String) -> Data? { inner.readerFont(name: name) }

    /// Tell the engine whether the machine currently has a connection.
    ///
    /// Reachability is a platform question, asked in the platform's own
    /// language (`Reachability`) and pushed down. Coming back from offline
    /// nudges a reconnect on the other side, so this is safe to call with the
    /// same answer repeatedly — `NWPathMonitor` does exactly that.
    public func setOffline(_ offline: Bool) { inner.setOffline(offline: offline) }

    /// Say which accounts the unified view can vouch for right now.
    ///
    /// Read when a whole-view selection is *made*, not when a verb runs
    /// (#811). The boundary's default is the empty set, which is safe and
    /// means `⌘A` in the unified list selects nothing until this is
    /// reported. See `VouchedFor` for what this frontend can honestly say.
    public func setReachableAccounts(_ accounts: [Int64]) {
        inner.setReachableAccounts(accounts: accounts)
    }

    /// Whether the platform has told the engine there is no connection.
    public var isOffline: Bool { inner.isOffline() }


    /// Everything the pane asks about one open message — the blocked-images
    /// notice, the decode caveat, the unsubscribe offer and the recipients —
    /// in one call, one body load and one render (#1589).
    ///
    /// **`nonisolated`.** The render inside it is the same sanitizer pass the
    /// reader runs, and the actor that draws must not pay for a render whose
    /// only output is a number. Call it from a task and publish the answer —
    /// which is exactly what `ExpandedMessage.task(id:)` does.
    public nonisolated func messageFacts(_ message: Int64) -> MessageFactsFfi {
        inner.messageFacts(message: message)
    }

    /// The whole document for a message, ready to hand a web view.
    ///
    /// Not fragments to assemble: the content security policy, the embedded
    /// font faces, the sanitized body inside its container and the scroll
    /// markers all come from the engine, which is what the GTK reader renders
    /// too. Swift composes no reader HTML.
    /// Every message opens as its sender built it (spec 006 FR-031);
    /// `reduced` is reader view, the reader's own choice for this message and
    /// this view (`⇧⌘O`). Nothing is remembered, so the next message opens as
    /// sent again.
    ///
    /// The answer carries the notice and the caveat too (#1589): both are
    /// by-products of the render the document pays for anyway, and asking
    /// for them separately was two more body loads.
    public func readerDocument(
        message: Int64,
        remote: RemoteImagesFfi,
        reduced: Bool = false
    ) -> ReaderDocumentFfi {
        inner.readerDocument(message: message, remote: remote, reduced: reduced)
    }

    /// One part's bytes.
    ///
    /// **`nonisolated`, and deliberately so.** This is the only call in the
    /// parts surface that can reach the network: a part nobody has
    /// downloaded is queued and then waited on for up to thirty seconds.
    /// Every other method here reads SQLite and answers in milliseconds.
    /// Calling it from the main actor would freeze the window on somebody's
    /// IMAP server, so the type system is asked to prevent that rather than a
    /// comment.
    public nonisolated func partBytes(_ message: Int64, partId: String) throws -> Data {
        Data(try inner.partBytes(message: message, partId: partId))
    }


    /// Leave the list this message came from — **the deliberate activation**,
    /// and the only call in this pair that records one.
    ///
    /// From a button and from nothing else. A message that offers nothing is
    /// refused rather than logged, so this cannot unsubscribe anybody from a
    /// message the reader never offered it on.
    ///
    /// **`nonisolated`**: it writes, and a write waits on the store's
    /// machine-wide gate — behind whatever the sync engine is committing,
    /// which on a first sync is not a few milliseconds.
    public nonisolated func activateUnsubscribe(_ message: Int64) -> String? {
        inner.activateUnsubscribe(message: message)
    }

    /// `always_show_images`: grant `message`'s sender a standing exception,
    /// if the reader is asking for one, and say whether it was. The address
    /// is the notice's own, so the key and the notice's "Always allow" cannot
    /// grant different people.
    ///
    /// **`nonisolated`**: it renders the message to learn what was held back.
    public nonisolated func alwaysShowImagesFor(_ message: Int64) -> Bool {
        inner.alwaysShowImagesFor(message: message)
    }

    /// Every activation this store holds, newest first — the Privacy pane's
    /// list. An action nobody can see afterwards is one nobody can audit.
    public func unsubscribeActivations() -> [UnsubscribeActivationFfi] {
        inner.unsubscribeActivations()
    }

    /// One inline part of `message`, by its `Content-ID`.
    ///
    /// `nil` when the bytes are not already on this machine — the privacy
    /// commitment rather than a gap to fill in later. Fetching here would be
    /// the tracking pixel the reader blocks, arriving through the back door.
    public func resolveCid(message: Int64, contentId: String) -> InlinePart? {
        inner.resolveCid(message: message, contentId: contentId)
    }

    /// `thread` as one document, with each message's anchor, sender and
    /// caveat (#1595, ADR 0032). The page is `postio_ui::reader::thread`'s,
    /// the same one GTK's pane draws. Blocks on the store: off the main
    /// actor.
    public nonisolated func threadDocument(thread: Int64, reduced: [Int64]) -> ThreadDocumentFfi {
        inner.threadDocument(thread: thread, reduced: reduced)
    }

    /// The draft behind a draft's message row, for a conversation's
    /// `Continue editing` — `nil` for another client's draft, which has
    /// nothing on this machine to edit.
    public func draftForMessage(_ message: Int64) -> DraftFfi? {
        inner.draftForMessage(message: message)
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

    /// Throw a draft away — the row and the server copy.
    ///
    /// Discarding one that is already gone is not an error: a retried
    /// discard, or one racing a send that already cleared the row, is the
    /// expected case, and the window has closed either way.
    @discardableResult
    public func discardDraft(_ draft: Int64) -> String? { inner.discardDraft(draft: draft) }

    /// Whether `id` can run in `context`, given the open view.
    ///
    /// What a menu asks before drawing an item enabled. The same question the
    /// palette's filter answers, so the two cannot disagree — before #1158
    /// only the palette was asking, and the menu drew Send and the whole
    /// Format menu as live options against a build with no composer.
    public func isAvailable(_ id: String, in context: UiContext) -> Bool {
        inner.isAvailable(id: id, context: context)
    }

    /// The key map as the controller would open it now (`focus_key_map`):
    /// what an open key map draws again after `[keys]` changes (T106).
    public func focusKeyMap() -> KeyMapSheetFfi { inner.focusKeyMap() }

    /// Put `label` on the selection, or on the message under the cursor when
    /// nothing is marked.
    public func applyLabel(_ label: Int64) { inner.applyLabel(label: label) }

    /// The cursor rested on `message` long enough for it to count as read.
    ///
    /// Not `invoke`: `MarkReadOnDwell` is deliberately outside the registry,
    /// because it is the one dispatch that is *not* recorded on the undo
    /// stack — `u` takes back what you did, and reading a mailbox produces
    /// one of these per message rested on.
    public func markReadOnDwell(_ message: Int64) {
        inner.markReadOnDwell(message: message)
    }

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

    /// What a rich body reads as in plain text, flowed.
    ///
    /// What the Rich/Plain switch needs at the moment it flips (#1293).
    /// Named for that call site rather than reusing `narrowPaste`, which
    /// returns the same string but would tell the next reader this was a
    /// paste.
    public func plainTextOf(_ html: String) -> String {
        inner.plainTextOf(html: html)
    }

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


    /// The verbs the reading pane offers, in canvas order.
    ///
    /// No key comes with them — `accelerator(for:)` is what spells one on
    /// this platform. What crosses is which verbs and in what order, which is
    /// the same on both frontends by construction.
    public func readerActions() -> [ReaderActionFfi] { inner.readerActions() }

    /// The header bar of a conversation of `messages`, each verb with what it
    /// will act on in words and whether that is the whole thread (FR-008,
    /// FR-008a).
    public func conversationActions(messages: UInt32) -> [ConversationActionFfi] {
        inner.conversationActions(messages: messages)
    }


    /// Always allow this address's remote images, across restarts.
    ///
    /// POSTIO-CONSENT: only ever from the popover's own item, which names
    /// the address it is about.
    public func allowSender(_ address: String) { inner.allowSender(address: address) }

    /// Always allow every address at this domain.
    public func allowDomain(_ domain: String) { inner.allowDomain(domain: domain) }

    /// A draft prefilled from a `mailto:` link.
    ///
    /// The parsing is `Mailto`'s — RFC 6068 is the platform's URL machinery —
    /// and what a mail client does with the result is the boundary's, so both
    /// frontends behave alike.
    public func mailtoDraft(_ mailto: Mailto) -> DraftFfi? {
        inner.mailtoDraft(
            to: mailto.to,
            cc: mailto.cc,
            bcc: mailto.bcc,
            subject: mailto.subject,
            body: mailto.body
        )
    }

    /// Every standing permission to load remote images.
    ///
    /// The Privacy pane's model. Blocked-until-allowed only means something
    /// if what has been allowed can be looked at and taken back.
    public func remoteImageGrants() -> [GrantFfi] {
        inner.remoteImageGrants()
    }

    /// Take one back; images from it are blocked again at once.
    public func revokeRemoteImages(_ subject: String) {
        inner.revokeRemoteImages(subject: subject)
    }

    /// What looking `address` up finds, for the first-run card (canvas 09).
    ///
    /// The desktop's lookup, with every connection it makes in the egress
    /// log. Waits on the network: not from the main actor, and never per
    /// keystroke.
    public func discoverAccount(_ address: String) -> DiscoveredFfi {
        inner.discoverAccount(address: address)
    }

    /// Sign in to the servers `account` names and, only if that works, save
    /// it. `nil` when it was added; a sentence when it was not. Waits on the
    /// server: not from the main actor.
    public func connectAccount(_ account: NewAccountFfi) -> String? {
        inner.connectAccount(account: account)
    }

    /// Add an account that is a directory on this machine.
    ///
    /// `nil` when it was added; a sentence when it was not. No credential is
    /// stored, because there is none: the account signs in to nothing.
    public func addLocalAccount(address: String, path: String) -> String? {
        inner.addLocalAccount(address: address, path: path)
    }

    /// Whether a directory is a mail store Postio can open.
    ///
    /// `nil` when it is; a sentence naming the directory when it is not. The
    /// sheet asks while somebody is still looking at the field, so what is
    /// wrong with a directory is said before an account points at it.
    public func inspectLocalStore(_ path: String) -> String? {
        inner.inspectLocalStore(path: path)
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

    /// Switch an account's syncing on or off.
    ///
    /// The row stays in the list: a disabled account is configured and not
    /// syncing, which is a state to show rather than one to hide.
    public func setAccountEnabled(_ account: Int64, _ enabled: Bool) -> String? {
        inner.setAccountEnabled(account: account, enabled: enabled)
    }

    /// Make an account the one new messages come from when the message
    /// itself does not say — and nothing else (#960).
    ///
    /// There is no way to clear it: the reversal of marking an account is
    /// marking another, which is why the command carries no undo.
    public func setDefaultAccount(_ account: Int64) -> String? {
        inner.setDefaultAccount(account: account)
    }

    /// Give an account a new password — the repair `AccountFfi.repair` calls
    /// `.password`.
    ///
    /// For the two states that leave an account unable to sign in with
    /// nothing wrong with its row: a provider that rotated its app password,
    /// and a row whose keyring entry never arrived. Neither is repaired by
    /// adding the account again.
    ///
    /// **Blocks on the keyring**, and on a locked one it blocks until
    /// somebody unlocks it. Never from the main actor.
    public nonisolated func repairCredential(_ account: Int64, _ password: String) -> String? {
        inner.repairCredential(account: account, password: password)
    }

    /// Sign an account in again through the system browser — the repair
    /// `AccountFfi.repair` calls `.browser`.
    ///
    /// Takes no client id, unlike a first sign-in: the account already
    /// carries the one it registered. Reconnect is one press rather than a
    /// form asking somebody to find a credential again in order to fix an
    /// account that used to work.
    ///
    /// **Returns when the flow is over**, which is when a person comes back
    /// from a browser tab. Never from the main actor.
    public nonisolated func reconnectAccount(_ account: Int64) -> String? {
        inner.reconnectAccount(account: account)
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
    /// Put a picture into the draft's body (#1571): answers the draft with
    /// the part on it, and the script that draws it at the caret.
    ///
    /// `bytes` rather than a file, because a picture arrives from a paste as
    /// often as from the open panel. Blocks on the store.
    public func insertImage(
        _ bytes: Data, mimeType: String, into draft: DraftFfi
    ) throws -> InlineImageFfi {
        try inner.insertInlineImage(draft: draft, bytes: bytes, mimeType: mimeType)
    }

    /// A picture in draft `draft`'s own body, by its `Content-ID` — what the
    /// composer's `postio-cid:` handler answers with.
    public func resolveDraftCid(draft: Int64, contentId: String) -> InlinePart? {
        inner.resolveDraftCid(draft: draft, contentId: contentId)
    }

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

    /// Queue a draft to leave at `when` — epoch milliseconds.
    ///
    /// Every check an immediate send makes, made here too: being refused at
    /// the scheduled hour, when nobody is watching the composer, is strictly
    /// worse than being refused now.
    public func sendDraftLater(_ draft: DraftFfi, at when: Int64) -> String? {
        inner.sendDraftLater(draft: draft, when: when)
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

extension PostioSession: FocusRowSource {
    /// How many rows Focus's list draws.
    public var focusRowCount: UInt32 { inner.focusRowCount() }

    /// The row at `position`, or `nil` while its page is on its way.
    /// Synchronous and no I/O: what the table asks for every visible row.
    public func focusRow(at position: UInt32) -> FocusRowFfi? {
        inner.focusRowAt(position: position)
    }
}
