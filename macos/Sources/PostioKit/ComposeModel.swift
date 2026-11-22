import Foundation
import PostioFFI

public enum RecipientField: Equatable, Sendable {
    case to, cc, bcc
}

/// Decides almost nothing: reply addressing, subject and quote are `postio_model::reply`'s, and
/// what leaves the machine is `postio_ui::compose`'s sentence. What is local is the unsaved state,
/// which must not be lost, so it is autosaved.
@MainActor
@Observable
public final class ComposeModel: Identifiable {
    public let id: Int64

    public private(set) var draft: DraftFfi

    public var to: String
    public var cc: String
    /// `edited` copied the draft wholesale, so a `bcc` that arrived on the draft (`mailto:?bcc=`)
    /// rode through to the send with nothing on screen showing it.
    public var bcc: String
    public var subject: String
    public var body: String

    /// Held beside `body`: the switch does not throw the other away. The editing surface reports it
    /// on every keystroke and the boundary narrows it on the way into the store, so this is a
    /// working copy, never the record (ADR 0004 Q3).
    public var bodyHtml: String?

    /// The switch is on the document: turning it off changes what is built from the words, not the
    /// words.
    public var rich: Bool

    public private(set) var status: String?

    public private(set) var sent = false

    public internal(set) var savedAt: Date?

    func markSent() { sent = true }

    public var heading: String { composerTitle(kind: draft.kind) }

    public var savedWords: String? {
        savedAt.map { draftSavedWords(at: Int64($0.timeIntervalSince1970 * 1000)) }
    }

    public var summary: String { draftSummary(draft: edited) }

    public var remindAt: Int64?

    public var remindWords: String { remindMeaning(at: remindAt) }

    public private(set) var labels: [DraftLabelFfi]

    public private(set) var labelsFromThread: Bool

    public func removeLabel(_ id: Int64) {
        labels.removeAll { $0.id == id }
        labelsFromThread = false
    }

    /// `body` holds only what is written above the quote and `edited` puts it back, so what is
    /// saved and sent still quotes.
    public private(set) var quoteFold: QuoteFoldFfi?

    public func showQuote() {
        guard let fold = quoteFold else { return }
        body += fold.quote
        quoteFold = nil
    }

    public var chooser: ChooserFfi? {
        didSet {
            // Takes the keyboard when it appears, not on each redraw with another card ringed.
            if chooser == nil { chooserHasKeyboard = false } else if oldValue == nil { chooserHasKeyboard = true }
        }
    }

    public private(set) var chooserHasKeyboard = false

    public func releaseChooser() { chooserHasKeyboard = false }

    public var olderReply: OlderReplyFfi?

    public var newer: [NewerMessageFfi]?

    public private(set) var quotedExpanded = false

    public var forwardedRows: [ForwardedRow] {
        quoteFold.map { ForwardedRows.rows(in: $0.quote) } ?? []
    }

    static func fold(_ body: String) -> QuoteFoldFfi? {
        ForwardedRows.fold(body) ?? foldQuote(body: body)
    }

    public func setQuotedExpanded(_ expanded: Bool) {
        quotedExpanded = expanded
        if expanded {
            showQuote()
        } else if !rich, quoteFold == nil, let fold = Self.fold(body) {
            body = fold.written
            quoteFold = fold
        }
    }

    /// Replace the body only when it differs, so the caret stays where a switch of the quote left
    /// what is written.
    public func adopt(_ new: DraftFfi) {
        var next = new
        next.from = draft.from
        next.path = draft.path
        next.labels = new.labels.map { label in
            labels.first { $0.id == label.id } ?? label
        }
        draft = next
        to = next.to
        cc = next.cc
        bcc = next.bcc
        showsCopyFields = showsCopyFields || !next.cc.isEmpty || !next.bcc.isEmpty
        subject = next.subject
        bodyHtml = next.bodyHtml
        rich = next.rich
        labels = next.labels
        let fold = next.rich ? nil : Self.fold(next.body)
        let written: String
        if let fold, !quotedExpanded {
            written = fold.written
            quoteFold = fold
        } else {
            written = next.body
            quoteFold = nil
        }
        if body != written { body = written }
        htmlAtSwitch = bodyHtml
        textAtSwitch = plainText
    }

    public private(set) var account: Int64
    public private(set) var from: String

    public func choose(account: AccountFfi) {
        self.account = account.id
        from = account.displayName.isEmpty
            ? account.address : "\(account.displayName) <\(account.address)>"
    }

    public private(set) var suggestions: [RecipientSuggestionFfi] = []
    public private(set) var suggesting: RecipientField?
    public private(set) var highlighted = 0

    public func text(of field: RecipientField) -> String {
        switch field {
        case .to: return to
        case .cc: return cc
        case .bcc: return bcc
        }
    }

    private func set(_ field: RecipientField, to text: String) {
        switch field {
        case .to: to = text
        case .cc: cc = text
        case .bcc: bcc = text
        }
    }

    public func suggest(_ answer: [RecipientSuggestionFfi], in field: RecipientField, for text: String) {
        guard self.text(of: field) == text else { return }
        suggestions = answer
        suggesting = answer.isEmpty ? nil : field
        highlighted = 0
    }

    public func moveSuggestion(by step: Int) {
        guard !suggestions.isEmpty else { return }
        highlighted = min(max(highlighted + step, 0), suggestions.count - 1)
    }

    @discardableResult
    public func acceptSuggestion() -> Bool {
        guard let field = suggesting, suggestions.indices.contains(highlighted) else { return false }
        set(field, to: suggestions[highlighted].accepted)
        dismissSuggestions()
        return true
    }

    public func dismissSuggestions() {
        suggestions = []
        suggesting = nil
        highlighted = 0
    }

    public init(id: Int64, draft: DraftFfi) {
        self.id = id
        self.draft = draft
        to = draft.to
        cc = draft.cc
        bcc = draft.bcc
        // Shown from the start when they hold somebody: `reply_all` puts every other original
        // recipient in `cc`, and a hidden one cannot be removed.
        showsCopyFields = !draft.cc.isEmpty || !draft.bcc.isEmpty
        subject = draft.subject
        bodyHtml = draft.bodyHtml
        rich = draft.rich
        account = draft.account
        from = draft.from
        remindAt = draft.remindAt
        labels = draft.labels
        labelsFromThread =
            draft.id == 0 && !draft.labels.isEmpty && (draft.kind == .reply || draft.kind == .replyAll)
        // A rich draft keeps its document whole: the editing surface is the one `editor.js` shared
        // with GTK.
        if !draft.rich, let fold = Self.fold(draft.body) {
            body = fold.written
            quoteFold = fold
        } else {
            body = draft.body
        }
        htmlAtSwitch = bodyHtml
        textAtSwitch = plainText
    }

    /// The subject is in the title bar so compose windows can be told apart in the Window menu.
    public var title: String {
        let trimmed = subject.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "New message" : trimmed
    }

    /// The path is abbreviated the way a person writes it (`~/Library/…`).
    public var footer: String {
        "draft in \(PostioPath.abbreviated(draft.path)) · \(outgoingShape(rich: sendsRich))"
    }

    /// `nil` until there is more than one person on it. A reply-to-all to a large list looks
    /// exactly like a reply until it is sent.
    public var recipientSummary: String? { PostioFFI.recipientSummary(draft: edited) }

    public var sendsRich: Bool { rich }

    public var marksApply: Bool { rich && !isHandedOff }

    /// Reported by the editing bridge on selection changes and edits, so a toolbar toggle reflects
    /// the document rather than guessing.
    public var caretMarks: Set<String> = []

    public func isMarkActive(_ command: String) -> Bool { caretMarks.contains(command) }

    /// The button lives in SwiftUI and the document in an `NSViewRepresentable`, so the press is
    /// left for the surface's next update.
    public struct MarkRequest: Equatable, Sendable {
        public let command: String
        /// Bold is a toggle: a request keyed only on the command would look unchanged the second
        /// time and the second press would be swallowed.
        public let serial: Int
        public let href: String?
    }

    public private(set) var markRequest: MarkRequest?
    private var marksAsked = 0

    /// A request, not a call: the address comes from a sheet, which a command cannot put up.
    public var wantsLink = false

    /// `Recovery::Confirm` in the registry: a command that discarded on the spot would be a
    /// destructive verb with no way back.
    public var wantsDiscard = false

    /// The file comes from an open panel, which a command cannot present.
    public var wantsAttachment = false

    public var wantsImage = false

    /// The part is already in the store and on the draft; what is left is running `script` in the
    /// document, once.
    public struct ImageRequest: Equatable, Sendable {
        public let script: String
        public let serial: Int
    }

    public private(set) var imageRequest: ImageRequest?
    private var imagesAsked = 0

    public var wantsRemind = false

    public var wantsSchedule = false

    /// Ignored on a plain draft: the keyboard reaches this too, and ⌘B over a document that cannot
    /// carry marks has nothing to apply to.
    public func applyMark(_ command: String, href: String? = nil) {
        guard marksApply else { return }
        marksAsked += 1
        markRequest = MarkRequest(command: command, serial: marksAsked, href: href)
    }

    /// "Whichever surface is active is authoritative" says nothing about the moment of the switch,
    /// when the field about to become authoritative is stale: everything typed in Rich went into
    /// the document, so without this Plain then send queued an empty message (#1293). Words already
    /// in the plain field are kept while the document is as Rich found it; once written in, the
    /// document is newer and `text` replaces them.
    public func switchedToPlain(text: String) {
        rich = false
        if body.isEmpty || bodyHtml != htmlAtSwitch {
            body = text
            quoteFold = nil
        }
        textAtSwitch = plainText
    }

    /// New drafts open plain, so turning Rich on met an empty document with the person's words
    /// gone. Marks are kept while the plain text is as it was; words written in Plain since are
    /// newer and replace them.
    public func switchedToRich(html: String) {
        rich = true
        if (bodyHtml ?? "").isEmpty || plainText != textAtSwitch {
            bodyHtml = html
        }
        htmlAtSwitch = bodyHtml
    }

    public var plainText: String { body + (quoteFold?.quote ?? "") }

    private var htmlAtSwitch: String?
    private var textAtSwitch: String?

    /// A plain draft has no document to hold a picture, and a handed-off one may not be written
    /// from this window. Both are said, since doing nothing is indistinguishable from broken.
    public func askForImage() {
        guard marksApply else {
            status = isHandedOff
                ? "This draft is open in another editor, so nothing can be added to its body here."
                : "A picture goes in the body of a rich message. Turn on Rich to put one there."
            return
        }
        wantsImage = true
    }

    public func insertImage(_ bytes: Data, mimeType: String, through session: PostioSession?) {
        guard let session, marksApply else { return }
        do {
            took(try session.insertImage(bytes, mimeType: mimeType, into: edited))
        } catch {
            status = error.localizedDescription
        }
    }

    public func insertImage(from file: URL, through session: PostioSession?) {
        let bytes: Data
        do {
            bytes = try Data(contentsOf: file)
        } catch {
            // The name is not in the sentence: it is the user's, and a status line is
            // screenshotted.
            status = "That picture could not be read."
            return
        }
        insertImage(bytes, mimeType: MimeType.ofImage(bytes) ?? MimeType.of(file), through: session)
    }

    public func took(_ inserted: InlineImageFfi) {
        draft = inserted.draft
        syncFromDraft()
        imagesAsked += 1
        imageRequest = ImageRequest(script: inserted.script, serial: imagesAsked)
        status = nil
    }

    public func refuseLink(_ href: String) {
        status = "A message can only link to http, https or mailto — not \(href)."
    }

    /// `nil` says nothing, the right answer when nothing was lost: announcing every paste would
    /// train people to ignore the one that mattered.
    public func tookPaste(_ pasted: PastedFfi) {
        // Only the sentence: the surface inserts the paste at the caret and the bridge reports the
        // document back on `input`, so writing the body here would replace everything typed.
        status = pasted.dropped
    }


    public var edited: DraftFfi {
        var edited = draft
        edited.to = to
        edited.cc = cc
        edited.bcc = bcc
        edited.subject = subject
        edited.body = plainText
        edited.bodyHtml = bodyHtml
        edited.rich = rich
        edited.account = account
        edited.from = from
        edited.remindAt = remindAt
        edited.labels = labels
        return edited
    }

    public private(set) var showsCopyFields: Bool = false

    /// They refuse to hide while they hold addresses (GTK's rule): hiding a field that holds
    /// somebody makes a recipient invisible.
    public func toggleCopyFields() {
        if showsCopyFields && (!cc.isEmpty || !bcc.isEmpty) { return }
        showsCopyFields.toggle()
    }

    /// `sent`, not a state of its own: the window must not autosave on its way out, which
    /// `onDisappear` would otherwise do, putting the discarded draft back.
    public func discard(through session: PostioSession) {
        if id != 0 || draft.id != 0 {
            session.discardDraft(draft.id)
        }
        sent = true
        status = nil
    }

    public var isDirty: Bool { edited != draft }

    /// `save` is idempotent on the id: a composer that forgot it would insert a new row on every
    /// autosave.
    public func save(through session: PostioSession) {
        _ = save { session.saveDraft($0) }
    }

    public static let notSaved = "This draft could not be saved."

    @discardableResult
    public func save(with write: (DraftFfi) -> DraftFfi?) -> Bool {
        guard let saved = write(edited) else {
            status = Self.notSaved
            return false
        }
        draft.id = saved.id
        draft.attachments = saved.attachments
        draft.path = saved.path
        draft.to = saved.to
        draft.cc = saved.cc
        draft.bcc = saved.bcc
        draft.subject = saved.subject
        draft.body = saved.body
        draft.bodyHtml = saved.bodyHtml
        draft.rich = saved.rich
        draft.remindAt = saved.remindAt
        draft.labels = saved.labels
        draft.account = saved.account
        draft.from = saved.from
        status = nil
        return true
    }

    /// Each is stored as it is taken and the draft saved, so an attachment survives the window
    /// closing.
    /// A file that will not attach stops that file only, and says which.
    public func attach(_ files: [URL], through session: PostioSession?) {
        guard let session, !files.isEmpty else { return }
        var refused: [String] = []
        for file in files {
            do {
                draft = try session.attach(file, to: edited)
                syncFromDraft()
            } catch {
                refused.append("\(file.lastPathComponent): \(error.localizedDescription)")
            }
        }
        status = refused.isEmpty ? nil : refused.joined(separator: "\n")
    }

    /// Every refusal `send` makes is made here too, by the boundary: being refused at 8am tomorrow,
    /// when nobody is watching, is worse than now.
    public func send(at when: Int64, through session: PostioSession?) {
        guard let session else { return }
        if let complaint = session.sendDraftLater(edited, at: when) {
            status = complaint
            return
        }
        status = nil
        sent = true
    }

    public func detach(_ attachment: AttachmentFfi, through session: PostioSession?) {
        guard let session else { return }
        do {
            draft = try session.detach(attachment.id, from: edited)
            syncFromDraft()
            status = nil
        } catch {
            status = error.localizedDescription
        }
    }

    public var attachments: [AttachmentFfi] { draft.attachments }

    /// Only what the store owns (the id and attachments): a save that overwrote the subject
    /// somebody was halfway through is unforgivable.
    private func syncFromDraft() {}

    public private(set) var handedOffTo: String?

    /// Another editor holds the draft; two writers would each silently undo the other.
    public var isHandedOff: Bool { handedOffTo != nil }

    public var editor = ""

    public func refreshEditor() {
        editor = ComposeHandoff.configuredEditor()
    }

    /// The draft is saved first, since an editor on a body Postio has not written down is one crash
    /// from being the only copy. Not `$EDITOR`: a Finder-launched app has no shell environment
    /// (#1288). A refusal takes the draft straight back, so the window is never left read-only
    /// waiting for an editor that never opened.
    public func handOff(
        through session: PostioSession?,
        open: (URL) async -> String?
    ) async {
        guard let session, !isHandedOff else { return }
        let path: String
        do {
            path = try session.beginHandoff(of: edited)
        } catch {
            status = error.localizedDescription
            return
        }
        draft = session.saveDraft(edited) ?? draft
        handedOffTo = path

        if let complaint = await open(URL(fileURLWithPath: path)) {
            status = complaint
            takeBack(through: session)
        } else {
            status = "Editing elsewhere. This window is read-only until you come back."
        }
    }

    public func takeBack(through session: PostioSession?) {
        guard let session, let path = handedOffTo else { return }
        do {
            draft = try session.endHandoff(of: edited, at: path)
            body = draft.body
            quoteFold = nil
            handedOffTo = nil
            status = nil
        } catch {
            // Left out there on purpose: the file is still the newer copy,
            // and a failed read that dropped the hand-off would strand it.
            status = error.localizedDescription
        }
    }

    /// A local write that `postio-sync` drains when there is a network. A refusal is the one case
    /// the window must stay open, because the words are in it.
    public func send(through session: PostioSession) -> Bool {
        if let complaint = session.sendDraft(edited) {
            status = complaint
            return false
        }
        sent = true
        status = nil
        return true
    }
}
