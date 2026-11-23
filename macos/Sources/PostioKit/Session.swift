import Foundation
import PostioFFI

/// Everything below is Rust. The app talks to this rather than the generated bindings, which are
/// regenerated every build. It adds no behaviour: decisions belong across the boundary, shared by
/// both frontends (ADR 0019).
/// Safe to hand across threads: the Rust type is `Send + Sync` (uniffi requires it), so a reader
/// document can be built off the main actor.
extension PostioSession: @unchecked Sendable {}

public final class PostioSession {
    private let inner: Session

    /// Call first: opening a session reads the Keychain and migrates the store, and both can fail
    /// before there is any UI to report it in.
    public static func startLogging() { PostioFFI.startLogging() }

    /// Blocks: the keyring round trip can wait on a user prompt, so never call on the main actor. A
    /// locked keyring arrives as `.keyringLocked`, distinct from a broken store.
    public static func open() throws -> PostioSession {
        PostioSession(inner: try Session.openAt(storePath: nil))
    }

    /// `false` for an unknown word, and always outside a demo build.
    public func demoState(_ state: String) -> Bool { inner.demoState(state: state) }

    /// In memory, reads no Keychain; refused by a build without demos.
    public static func openDemo(_ seed: String) throws -> PostioSession {
        PostioSession(inner: try Session.openDemo(seed: seed))
    }

    private init(inner: Session) {
        self.inner = inner
    }

    public var isOpen: Bool { inner.isOpen() }

    public var commands: [CommandSpecFfi] { inner.commands() }

    /// Flat, with parent ids; the tree is rebuilt for display.
    public var mailboxes: [MailboxFfi] { inner.mailboxes() }

    /// Read from here, not `config.toml`, so the list and the settings window cannot hold two
    /// opinions of the table.
    public func appearance() -> AppearanceFfi { inner.appearance() }

    public func searchHints() -> [KeyHintFfi] { inner.searchHints() }
    public func accounts() -> [AccountFfi] { inner.accounts() }

    /// Returns at once: `UiEvent.conversationReady` says when there is something to draw. The list
    /// is untouched; no drill-in on either platform.
    public func openConversation(_ thread: Int64) { inner.openConversation(thread: thread) }

    /// `nil` until one has been asked for. Which messages open is not Swift's to decide: it costs
    /// one web view each, and `postio_ui::conversation` answers it for both frontends.
    public var conversation: ConversationFfi? { inner.conversation() }

    public func openFocus(_ scope: FocusScopeFfi) { inner.openFocus(scope: scope) }

    /// Nothing moves until the controller answers with `FocusCursor`.
    public func focusPoint(_ position: Int) {
        guard let row = UInt32(exactly: position) else { return }
        inner.focusPoint(position: row)
    }

    public func focusChevronClicked(_ position: UInt32) {
        inner.focusChevronClicked(position: position)
    }

    public func focusActivate(_ position: Int) {
        guard let row = UInt32(exactly: position) else { return }
        inner.focusActivate(position: row)
    }

    public func focusPick(_ position: Int, range: Bool) {
        guard let row = UInt32(exactly: position) else { return }
        inner.focusPick(position: row, range: range)
    }

    /// Where an undo that brings rows in above leaves it.
    public func focusAtTop(_ atTop: Bool) { inner.focusAtTop(atTop: atTop) }

    public func focusBarRoom(_ chips: UInt32) { inner.focusBarRoom(chips: chips) }

    public func focusThreadBarChipClicked(_ index: UInt32) {
        inner.focusThreadBarChipClicked(index: index)
    }

    public func focusThreadBarMoreClicked(from: UInt32) {
        inner.focusThreadBarMoreClicked(from: from)
    }

    public func focusThreadMenuPick(command: String) { inner.focusThreadMenuPick(command: command) }

    public func focusOutlineTyped(text: String) { inner.focusOutlineTyped(text: text) }

    public func focusOutlinePerson(name: String?) { inner.focusOutlinePerson(name: name) }

    public func focusOutlineMove(by step: Int32) { inner.focusOutlineMove(by: step) }

    public func focusOutlineShow(row: UInt32?) { inner.focusOutlineShow(row: row) }

    public func focusOutlineFocusFilter() { inner.focusOutlineFocusFilter() }

    public func demoArrive(what: String) -> Bool { inner.demoArrive(what: what) }

    /// A store read: off the main actor (`PostioUndoManager.refresh`).
    public nonisolated func undoDescription() -> String? { inner.undoDescription() }

    public func focusCounts() throws -> FocusCountsFfi { try inner.focusCounts() }

    public func focusStrip() throws -> FocusStripFfi { try inner.focusStrip() }

    public func notices() -> NoticesFfi { inner.notices() }

    /// On the Mac it replaces the secondary window that was open.
    public func focusSurfaceOpened(_ kind: SurfaceKindFfi) { inner.focusSurfaceOpened(kind: kind) }

    public func focusSurfaceClosed(_ kind: SurfaceKindFfi) { inner.focusSurfaceClosed(kind: kind) }

    public func focusComposerEdited() { inner.focusComposerEdited() }

    public func focusComposerDraft(edit: DraftEditFfi, draft: DraftFfi) {
        inner.focusComposerDraft(edit: edit, draft: draft)
    }

    public func focusForwardFormClicked(_ form: ForwardFormFfi) {
        inner.focusForwardFormClicked(form: form)
    }

    /// Read from the store; nothing is fetched.
    public func focusNewerMessages(_ ids: [Int64]) -> [NewerMessageFfi] {
        inner.focusNewerMessages(ids: ids)
    }

    public func focusDraftSaved(_ composition: UInt64, kept: Bool, error: String?) {
        inner.focusDraftSaved(composition: composition, kept: kept, error: error)
    }

    /// Blocks on the store: off the main actor.
    public nonisolated func recipientSuggestions(
        account: Int64, text: String, limit: UInt32, extra: [ExternalContactFfi]
    ) -> [RecipientSuggestionFfi] {
        inner.recipientSuggestions(account: account, text: text, limit: limit, extra: extra)
    }

    public func focusBarTyped(_ text: String) { inner.focusBarTyped(text: text) }

    public func focusBarRun(_ token: UInt64) { inner.focusBarRun(token: token) }

    public func focusBarTab() -> Bool { inner.focusBarTab() }

    public func focusSearchHighlighted(_ token: UInt64) { inner.focusSearchHighlighted(token: token) }

    public func focusSearchForget(_ token: UInt64) { inner.focusSearchForget(token: token) }

    public func focusSearchExclude(_ token: UInt64) { inner.focusSearchExclude(token: token) }

    public func focusSearchShowAll() { inner.focusSearchShowAll() }

    public func focusSearchPlaceholder() -> String { inner.focusSearchPlaceholder() }
    public func focusSearchRestingPlaceholder() -> String { inner.focusSearchRestingPlaceholder() }

    public func focusSearchEdit(_ edit: TermEditFfi) { inner.focusSearchEdit(edit: edit) }

    public func focusSearchPopover(_ kind: FilterKindFfi) { inner.focusSearchPopover(kind: kind) }

    public func focusSearchPopoverToggle(_ token: UInt64, exclude: Bool) {
        inner.focusSearchPopoverToggle(token: token, exclude: exclude)
    }

    public func focusSearchPopoverFilter(_ text: String) { inner.focusSearchPopoverFilter(text: text) }

    public func focusSearchPopoverDone(_ apply: Bool) { inner.focusSearchPopoverDone(apply: apply) }

    public func focusSearchDateWords(_ text: String) { inner.focusSearchDateWords(text: text) }

    public func focusSearchDatePreset(_ token: UInt64) { inner.focusSearchDatePreset(token: token) }

    public func focusSearchMonths(_ first: UInt32, _ last: UInt32) {
        inner.focusSearchMonths(first: first, last: last)
    }

    public func focusSearchTab(_ tab: ResultsTabFfi) { inner.focusSearchTab(tab: tab) }

    public func focusSearchOrder(_ order: ConversationOrderFfi) { inner.focusSearchOrder(order: order) }

    public func focusSearchPoint(_ position: UInt64) { inner.focusSearchPoint(position: position) }

    public func focusSearchSave(_ name: String, pin: Bool, notify: Bool, rolling: Bool) {
        inner.focusSearchSave(name: name, pin: pin, notify: notify, rolling: rolling)
    }

    public func focusSearchRowCount() -> UInt64 { inner.focusSearchRowCount() }

    /// `nil` while its page is on its way; read again on `FocusResultsPage`.
    public func focusSearchRow(_ position: UInt64) -> ResultRowFfi? { inner.focusSearchRow(position: position) }

    public func focusSearchFile(_ position: UInt64) -> FileCardFfi? { inner.focusSearchFile(position: position) }

    public func focusSearchPerson(_ position: UInt64) -> PersonRowFfi? { inner.focusSearchPerson(position: position) }

    /// The copy handed to Quick Look or the save panel can go once it is gone.
    public func focusSearchFileDone() { inner.focusSearchFileDone() }

    public func focusPlaces(_ filter: String) -> [PlaceEntryFfi] { inner.focusPlaces(filter: filter) }

    public func focusOpenPlace(_ token: UInt64) { inner.focusOpenPlace(token: token) }

    public func focusPlacesPlaceholder() -> String { inner.focusPlacesPlaceholder() }

    public func focusPickerTyped(_ text: String) { inner.focusPickerTyped(text: text) }

    public func focusPickerChoose(_ token: UInt64) { inner.focusPickerChoose(token: token) }

    public func focusPickerToggle(_ token: UInt64) { inner.focusPickerToggle(token: token) }

    public func focusFilteredPoint(_ index: UInt32) { inner.focusFilteredPoint(index: index) }

    public func focusFilteredPick(_ index: UInt32, range: Bool) {
        inner.focusFilteredPick(position: index, range: range)
    }

    public func focusFilteredMore() { inner.focusFilteredMore() }

    public func focusDigestPoint(_ index: UInt32) { inner.focusDigestPoint(index: index) }

    public func focusDigestReference(_ index: UInt32) { inner.focusDigestReference(index: index) }

    public func focusConfirmed(_ token: UInt64) { inner.focusConfirmed(token: token) }

    public func focusRuleQuery(_ text: String) { inner.focusRuleQuery(text: text) }

    public func focusRuleMatchInstead() { inner.focusRuleMatchInstead() }

    public func focusRuleLikeThis() { inner.focusRuleLikeThis() }

    public func focusRuleSchedule(_ schedule: RuleScheduleFfi) { inner.focusRuleSchedule(schedule: schedule) }

    public func focusRuleCreate() { inner.focusRuleCreate() }

    /// `nil` while its page is on its way.
    public func focusContactsRow(_ index: UInt32) -> ContactRowViewFfi? {
        inner.focusContactsRow(index: index)
    }

    public func focusContactsTotal() -> UInt32 { inner.focusContactsTotal() }

    public func focusContactsCursor() -> UInt32? { inner.focusContactsCursor() }

    public func focusContactsCard() -> ContactCardViewFfi? { inner.focusContactsCard() }

    public func focusContactsTab(_ tab: ContactTabFfi) { inner.focusContactsTab(tab: tab) }

    public func focusContactsFilter(_ text: String) { inner.focusContactsFilter(text: text) }

    public func focusContactsPoint(_ index: UInt32) { inner.focusContactsPoint(index: index) }

    public func focusContactMergeKeep(_ index: UInt32) { inner.focusContactMergeKeep(index: index) }

    public func focusContactSplit(_ address: String) { inner.focusContactSplit(address: address) }

    public func contactsPanelWords() -> ContactsPanelWordsFfi { inner.contactsPanelWords() }

    public func contactsImport(_ bytes: Data) -> ContactsImportResultFfi {
        inner.contactsImport(bytes: bytes)
    }

    public func contactsExport() -> ContactsExportResultFfi { inner.contactsExport() }

    public func demoContactsFile() -> Data { inner.demoContactsFile() }

    public func contactsPane(
        _ scope: SyncScopeFfi, _ mode: SyncModeFfi, _ status: SyncStatusFfi, _ plan: SyncPlanFfi?
    ) -> ContactsPaneFfi {
        inner.contactsPane(scope: scope, mode: mode, status: status, plan: plan)
    }

    public func contactsSyncPlan(
        _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi
    ) -> SyncResultFfi {
        inner.contactsSyncPlan(records: records, scope: scope, mode: mode)
    }

    /// Postio's database only, never the address book.
    public func contactsSyncBringIn(
        _ records: [ExternalRecordFfi], _ scope: SyncScopeFfi, _ mode: SyncModeFfi,
        _ fingerprint: UInt64
    ) -> BringInResultFfi {
        inner.contactsSyncBringIn(records: records, scope: scope, mode: mode, fingerprint: fingerprint)
    }

    public func focusContactsSettingsOpened() { inner.focusContactsSettingsOpened() }

    public func focusContactsSuggestion(_ index: UInt32) { inner.focusContactsSuggestion(index: index) }

    public func focusContactsSettings() -> SuggestionsViewFfi? { inner.focusContactsSettings() }

    public func focusContactEdit() -> ContactEditViewFfi? { inner.focusContactEdit() }

    public func focusContactEditChange(_ change: EditChangeFfi) {
        inner.focusContactEditChange(change: change)
    }

    public func focusContactCardSwitch(_ index: UInt32) {
        inner.focusContactCardSwitch(index: index)
    }

    public func focusContactPopoverAt(_ address: String) {
        inner.focusContactPopoverAt(address: address)
    }

    public func focusContactArrival(_ arrival: ContactArrivalFfi) {
        inner.focusContactArrival(arrival: arrival)
    }

    public func focusCaptureTyped(_ text: String) { inner.focusCaptureTyped(text: text) }

    /// Capture's due day, "YYYY-MM-DD", or `nil` for none.
    public func focusCaptureDue(_ day: String?) { inner.focusCaptureDue(day: day) }

    public func focusCaptureFilter(_ text: String) { inner.focusCaptureFilter(text: text) }

    public func focusCaptureProject(_ token: UInt64) { inner.focusCaptureProject(token: token) }

    public func focusOpenLink(uri: String) { inner.focusOpenLink(uri: uri) }

    public func focusReaderState(moreOpen: Bool, finding: Bool) {
        inner.focusReaderState(moreOpen: moreOpen, finding: finding)
    }

    public func focusMatchEdge(forward: Bool) { inner.focusMatchEdge(forward: forward) }

    /// `nil` forgets the choice.
    public func alwaysTreatment(sender: String, treatment: TreatmentFfi?) {
        inner.alwaysTreatment(sender: sender, treatment: treatment)
    }

    /// A store read; never on the main actor.
    public nonisolated func focusMessageView(
        message: Int64, index: UInt32, total: UInt32
    ) -> FocusMessageViewFfi {
        inner.focusMessageView(message: message, index: index, total: total)
    }

    /// A store read; never on the main actor.
    public nonisolated func focusReaderDocument(
        message: Int64, remote: RemoteImagesFfi, chosen: TreatmentFfi?, mainWidth: Int32
    ) -> FocusReaderDocumentFfi {
        inner.focusReaderDocument(
            message: message, remote: remote, chosen: chosen, mainWidth: mainWidth)
    }

    /// May fetch from the server, the person having asked for these bytes by name: never on the
    /// main actor.
    public nonisolated func rawSource(_ message: Int64) throws -> Data {
        try inner.rawSource(message: message)
    }

    public nonisolated func readerFont(_ name: String) -> Data? { inner.readerFont(name: name) }

    /// Safe to call repeatedly with the same answer: coming back online nudges a reconnect on the
    /// other side, and `NWPathMonitor` repeats itself.
    public func setOffline(_ offline: Bool) { inner.setOffline(offline: offline) }

    /// Read when a whole-view selection is made, not when a verb runs (#811). The default is the
    /// empty set, so `⌘A` in the unified list selects nothing until this is reported.
    public func setReachableAccounts(_ accounts: [Int64]) {
        inner.setReachableAccounts(accounts: accounts)
    }

    public var isOffline: Bool { inner.isOffline() }


    /// One call, one body load and one render for everything the pane asks about a message (#1589).
    /// `nonisolated`: call from a task and publish the answer; the drawing actor must not pay for a
    /// render.
    public nonisolated func messageFacts(_ message: Int64) -> MessageFactsFfi {
        inner.messageFacts(message: message)
    }

    /// The whole document comes from the engine (CSP, fonts, sanitized body, scroll markers); Swift
    /// composes no reader HTML. `reduced` is reader view for this message and view only (`⇧⌘O`).
    /// The notice and caveat are by-products of the same render (#1589).
    public func readerDocument(
        message: Int64,
        remote: RemoteImagesFfi,
        reduced: Bool = false
    ) -> ReaderDocumentFfi {
        inner.readerDocument(message: message, remote: remote, reduced: reduced)
    }

    /// `nonisolated`: the only call here that can reach the network (a part not downloaded is
    /// queued and awaited up to thirty seconds); the main actor must never call it.
    public nonisolated func partBytes(_ message: Int64, partId: String) throws -> Data {
        Data(try inner.partBytes(message: message, partId: partId))
    }


    /// The deliberate activation, and the only call in this pair that records one. A message that
    /// offers nothing is refused, so no one is unsubscribed from a message the reader never offered
    /// it on. `nonisolated`: it writes behind the store's machine-wide gate.
    public nonisolated func listMailOffer(_ message: Int64) -> ListMailOfferFfi? {
        inner.listMailOffer(message: message)
    }

    public nonisolated func clearListMail(_ message: Int64, _ delete: Bool) -> String? {
        inner.clearListMail(message: message, delete: delete)
    }

    public nonisolated func openChoices(_ message: Int64, _ document: String) -> OpenChoicesFfi {
        inner.openChoices(message: message, document: document)
    }

    /// Never on the main actor, as `openAttachment`.
    public nonisolated func saveAttachment(_ message: Int64, _ attachment: Int64, to: String) throws -> String {
        try inner.saveAttachment(message: message, attachment: attachment, to: to)
    }

    /// Never on the main actor, as `openAttachment`.
    public nonisolated func saveAttachments(_ message: Int64, folder: String) throws -> String {
        try inner.saveAttachments(message: message, folder: folder)
    }

    /// Never on the main actor; it may reach the server for bytes the person asked for by name.
    public nonisolated func openAttachment(_ message: Int64, _ attachment: Int64) throws -> String {
        try inner.openAttachment(message: message, attachment: attachment)
    }

    public nonisolated func activateUnsubscribe(_ message: Int64) -> String? {
        inner.activateUnsubscribe(message: message)
    }

    /// The address is the notice's own, so the key and the notice's "Always allow" cannot grant
    /// different people. `nonisolated`: it renders the message to learn what was held back.
    public nonisolated func alwaysShowImagesFor(_ message: Int64) -> Bool {
        inner.alwaysShowImagesFor(message: message)
    }

    /// `nonisolated`: reads the access log. Call from a task and publish the answer.
    public nonisolated func agentActivity() -> AgentActivityFfi {
        inner.agentActivity()
    }

    public func unsubscribeActivations() -> [UnsubscribeActivationFfi] {
        inner.unsubscribeActivations()
    }

    /// `nil` when the bytes are not already on this machine: fetching here would be the tracking
    /// pixel the reader blocks.
    public func resolveCid(message: Int64, contentId: String) -> InlinePart? {
        inner.resolveCid(message: message, contentId: contentId)
    }

    /// Blocks on the store: off the main actor.
    public nonisolated func threadDocument(thread: Int64, reduced: [Int64]) -> ThreadDocumentFfi {
        inner.threadDocument(thread: thread, reduced: reduced)
    }

    /// `nil` for another client's draft, which has nothing on this machine to edit.
    public func draftForMessage(_ message: Int64) -> DraftFfi? {
        inner.draftForMessage(message: message)
    }

    /// The whole of the app's keyboard, and it decides nothing: `postio_ui::keymap` owns the table,
    /// chords, sequences and leader timeout for both frontends (ADR 0019 Q4). `typing` is whether
    /// the focused surface takes text, which only the caller can see.
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

    /// Nothing comes back: a verb writes to SQLite, enqueues and returns, and what happened arrives
    /// on `nextEvent`. The UI never awaits the network.
    public func invoke(_ id: String) { inner.invoke(id: id) }

    /// Discarding one already gone is not an error: a retried discard, or one racing a send that
    /// cleared the row, is expected.
    @discardableResult
    public func discardDraft(_ draft: Int64) -> String? { inner.discardDraft(draft: draft) }

    /// The same question the palette's filter answers, so the menu and palette cannot disagree.
    public func isAvailable(_ id: String, in context: UiContext) -> Bool {
        inner.isAvailable(id: id, context: context)
    }

    public func focusKeyMap() -> KeyMapSheetFfi { inner.focusKeyMap() }

    public func applyLabel(_ label: Int64) { inner.applyLabel(label: label) }

    /// Not `invoke`: `MarkReadOnDwell` is outside the registry because it is not recorded on the
    /// undo stack.
    public func markReadOnDwell(_ message: Int64) {
        inner.markReadOnDwell(message: message)
    }

    /// The user's override, else the default, resolved for this platform (`cmd+k`, not the table's
    /// `mod+k`).
    public func binding(for command: String) -> String? {
        inner.bindingFor(command: command)
    }

    /// The one place a key is turned into glyphs, so one button cannot say `⌘R` beside another
    /// saying `E`. Both keyboard layers are live: the `⌘` chord, falling back to the mnemonic.
    public func accelerator(for command: String) -> String? {
        MenuPlan.accelerator(among: bindings(for: command))
    }

    /// Both keyboard layers are two bindings on one command, not a mode: `e` replies and so does
    /// `⌘R`.
    public func bindings(for command: String) -> [String] {
        inner.bindingsFor(command: command)
    }

    /// `nil` when nothing is held back: a notice with nothing to report teaches people to dismiss
    /// the one that matters.
    /// Who `message` was addressed to; `nil` for a message the store does not hold. A read of its
    /// own: the list draws no recipients.
    /// `postio_ui::compose::EDITOR_SCRIPT`, crossing rather than copied because it decides the
    /// dialect the surface emits; a second copy would produce `<div>`s where this produces `<p>`s.
    public func editorScript() -> String { inner.editorScript() }

    /// Named for the Rich/Plain flip (#1293) rather than reusing `narrowPaste`, which would suggest
    /// a paste.
    public func plainTextOf(_ html: String) -> String {
        inner.plainTextOf(html: html)
    }

    public func richHtmlOf(_ text: String) -> String {
        inner.richHtmlOf(text: text)
    }

    /// `nil` for a command that is not one of the marks.
    public func markScript(_ command: String) -> String? {
        inner.markScript(command: command)
    }

    /// `nil` when a message may not point there.
    public func linkScript(_ href: String) -> String? {
        inner.linkScript(href: href)
    }

    /// Pure: no store, no network. The sentence in `dropped` is the engine's, so both composers
    /// agree.
    public func narrowPaste(_ html: String) -> PastedFfi {
        inner.narrowPaste(html: html)
    }

    public func draft(_ id: Int64) -> DraftFfi? { inner.draft(id: id) }


    /// No key comes with them: `accelerator(for:)` spells one on this platform.
    public func readerActions() -> [ReaderActionFfi] { inner.readerActions() }

    public func conversationActions(messages: UInt32) -> [ConversationActionFfi] {
        inner.conversationActions(messages: messages)
    }


    /// Always allow this address's remote images, across restarts.
    ///
    /// POSTIO-CONSENT: only ever from the popover's own item, which names
    /// the address it is about.
    public func allowSender(_ address: String) { inner.allowSender(address: address) }

    public func allowDomain(_ domain: String) { inner.allowDomain(domain: domain) }

    /// Parsing is `Mailto`'s (RFC 6068); what a client does with the result is the boundary's.
    public func mailtoDraft(_ mailto: Mailto) -> DraftFfi? {
        inner.mailtoDraft(
            to: mailto.to,
            cc: mailto.cc,
            bcc: mailto.bcc,
            subject: mailto.subject,
            body: mailto.body
        )
    }

    public func remoteImageGrants() -> [GrantFfi] {
        inner.remoteImageGrants()
    }

    public func revokeRemoteImages(_ subject: String) {
        inner.revokeRemoteImages(subject: subject)
    }

    /// Every connection it makes is in the egress log. Waits on the network: not from the main
    /// actor, and never per keystroke.
    public func discoverAccount(_ address: String) -> DiscoveredFfi {
        inner.discoverAccount(address: address)
    }

    /// `nil` when added; a sentence when not. Waits on the server: not from the main actor.
    public func connectAccount(_ account: NewAccountFfi) -> String? {
        inner.connectAccount(account: account)
    }

    /// `nil` when added; a sentence when not. No credential is stored: the account signs in to
    /// nothing.
    public func addLocalAccount(address: String, path: String) -> String? {
        inner.addLocalAccount(address: address, path: path)
    }

    /// `nil` when it is; a sentence naming the directory when not. Asked while the field is still
    /// in view.
    public func inspectLocalStore(_ path: String) -> String? {
        inner.inspectLocalStore(path: path)
    }

    /// Blocks, so use a detached task. It takes sync's path (same credential, same settings), so a
    /// passing test is a statement about sync.
    public func testConnection(_ account: Int64) -> ConnectionReportFfi {
        inner.testConnection(account: account)
    }

    /// The one field the account form edits: everything else came from the preset table or a
    /// sign-in.
    public func setDisplayName(_ account: Int64, to name: String) -> String? {
        inner.setDisplayName(account: account, name: name)
    }

    /// `nil` when there is nothing to weigh: a fresh account says nothing rather than `0 B`.
    public func weight(of account: Int64) -> String? {
        inner.accountWeight(account: account)
    }

    /// Rebuild this account's search index from the mail already here.
    /// Blocks, and reaches no server.
    public func reindexAccount(_ account: Int64) -> String? {
        inner.reindexAccount(account: account)
    }

    /// The row stays: a disabled account is configured and not syncing, a state to show.
    public func setAccountEnabled(_ account: Int64, _ enabled: Bool) -> String? {
        inner.setAccountEnabled(account: account, enabled: enabled)
    }

    /// No way to clear it: the reversal is marking another account, so the command carries no undo
    /// (#960).
    public func setDefaultAccount(_ account: Int64) -> String? {
        inner.setDefaultAccount(account: account)
    }

    /// For an account that cannot sign in with nothing wrong with its row (a rotated app password,
    /// a keyring entry that never arrived). Blocks on the keyring, until unlocked if locked. Never
    /// from the main actor.
    public nonisolated func repairCredential(_ account: Int64, _ password: String) -> String? {
        inner.repairCredential(account: account, password: password)
    }

    /// Takes no client id: the account carries the one it registered. Returns when the flow is
    /// over, when a person comes back from a browser tab. Never from the main actor.
    public nonisolated func reconnectAccount(_ account: Int64) -> String? {
        inner.reconnectAccount(account: account)
    }

    /// Removes its row and its credentials.
    public func removeAccount(_ account: Int64) -> String? {
        inner.removeAccount(account: account)
    }

    /// Blocks until the flow is over (a person in another application), so use a detached task. The
    /// client id is the user's own: Postio ships none (ADR 0006 Q1), since a credential in an
    /// open-source app is shared by every user.
    public func signInWithBrowser(
        address: String,
        clientId: String,
        clientSecret: String?
    ) -> String? {
        inner.signInWithBrowser(
            address: address, clientId: clientId, clientSecret: clientSecret)
    }

    public var signInProgress: SignInProgressFfi { inner.signInProgress() }

    public func cancelSignIn() { inner.cancelSignIn() }

    /// `nil` when no account is configured, a real state on a fresh install.
    public func newDraft() -> DraftFfi? { inner.newDraft() }

    /// Addressing, subject and quote are `postio_model::reply`'s; Swift addresses nothing.
    public func replyDraft(to message: Int64, all: Bool) -> DraftFfi? {
        inner.replyDraft(message: message, all: all)
    }

    public func forwardDraft(_ message: Int64) -> DraftFfi? {
        inner.forwardDraft(message: message)
    }

    /// The MIME type is sniffed here because it is a platform service (`UniformTypeIdentifiers`
    /// here, shared-mime-info on freedesktop). The size guard, blob write and row happen once,
    /// across the boundary.
    public func attach(_ file: URL, to draft: DraftFfi) throws -> DraftFfi {
        try inner.attachToDraft(
            draft: draft,
            path: file.path,
            mimeType: MimeType.of(file)
        )
    }

    /// The file is private (a private directory, mode 0600): an unsent draft is often the most
    /// private mail.
    public func beginHandoff(of draft: DraftFfi) throws -> String {
        try inner.beginHandoff(draft: draft)
    }

    public func endHandoff(of draft: DraftFfi, at path: String) throws -> DraftFfi {
        try inner.endHandoff(draft: draft, path: path)
    }

    /// `bytes` rather than a file, because a picture arrives from a paste as often as from the open
    /// panel. Blocks on the store.
    public func insertImage(
        _ bytes: Data, mimeType: String, into draft: DraftFfi
    ) throws -> InlineImageFfi {
        try inner.insertInlineImage(draft: draft, bytes: bytes, mimeType: mimeType)
    }

    public func resolveDraftCid(draft: Int64, contentId: String) -> InlinePart? {
        inner.resolveDraftCid(draft: draft, contentId: contentId)
    }

    public func detach(_ attachment: Int64, from draft: DraftFfi) throws -> DraftFfi {
        try inner.detachFromDraft(draft: draft, attachment: attachment)
    }

    public func saveDraft(_ draft: DraftFfi) -> DraftFfi? {
        inner.saveDraft(draft: draft)
    }

    /// `nil` when it went; a sentence when not, which the composer shows rather than closing over.
    public func sendDraft(_ draft: DraftFfi) -> String? {
        inner.sendDraft(draft: draft)
    }

    /// `when` is epoch milliseconds. Every check an immediate send makes is made here too: being
    /// refused at the scheduled hour, unwatched, is worse.
    public func sendDraftLater(_ draft: DraftFfi, at when: Int64) -> String? {
        inner.sendDraftLater(draft: draft, when: when)
    }

    @discardableResult
    public func startSyncing() throws -> UInt32 { try inner.startSyncing() }

    public var configuredAccounts: UInt32 { inner.configuredAccounts() }

    public func shutdown() { inner.shutdown() }

    /// `nil` once the session has stopped. Drive it on the main actor, the same drain GTK runs on
    /// its main context, so no backend work reaches the UI thread.
    public func nextEvent() async -> UiEvent? { await inner.nextEvent() }
}

extension PostioSession: FocusRowSource {
    public var focusRowCount: UInt32 { inner.focusRowCount() }

    /// Synchronous, no I/O: what the table asks for every visible row.
    public func focusRow(at position: UInt32) -> FocusRowFfi? {
        inner.focusRowAt(position: position)
    }

    public func residentFocusRow(at position: UInt32) -> FocusRowFfi? {
        inner.focusResidentRowAt(position: position)
    }
}
