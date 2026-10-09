import Foundation
import PostioFFI

/// A recipient field, for completion.
public enum RecipientField: Equatable, Sendable {
    case to, cc, bcc
}

/// One message being written (#1272, canvas screen 26).
///
/// Holds the draft the boundary handed over and whatever has been typed
/// since. It decides almost nothing: who a reply is addressed to, what its
/// subject became and what the quote says are `postio_model::reply`'s
/// answers, and what will leave the machine is `postio_ui::compose`'s
/// sentence. What is local is the unsaved state — which is exactly the part
/// that must not be lost, so it is autosaved rather than held.
@MainActor
@Observable
public final class ComposeModel: Identifiable {
    /// This window's own id, so several compose windows can be open at once
    /// and the Window menu can tell them apart.
    public let id: Int64

    /// The draft, as last agreed with the store.
    public private(set) var draft: DraftFfi

    /// What has been typed since. Separate from `draft` so a save can tell
    /// what it is saving.
    public var to: String
    public var cc: String
    /// The blind copies.
    ///
    /// **It was not here at all**, and `edited` copied the draft wholesale
    /// and overwrote only the fields that were — so a `bcc` that arrived on
    /// the draft (a `mailto:?bcc=…`, or anything that built it before the
    /// window opened) rode through to the send with nothing on screen showing
    /// it and no way to take it off.
    public var bcc: String
    public var subject: String
    public var body: String

    /// The body as marked-up text, when this draft is rich (#1271).
    ///
    /// Held beside `body` rather than instead of it, because the switch does
    /// not throw the other one away: a draft can be plain and still be
    /// holding the marks it had a moment ago, and turning Rich back on
    /// should cost nothing.
    ///
    /// What the editing surface reports on every keystroke, and what the
    /// boundary narrows to the dialect on the way into the store -- so this
    /// is a working copy and never the record (ADR 0004 Q3).
    public var bodyHtml: String?

    /// Whether this is being written as rich text.
    ///
    /// The switch is on the document, not on the window: turning it off does
    /// not throw the words away, it changes what will be built out of them.
    public var rich: Bool

    /// Anything the composer has to say — a refusal to send, mostly.
    public private(set) var status: String?

    /// Whether the window may close without asking.
    public private(set) var sent = false

    /// When what is written was last saved: the title's "Draft saved
    /// locally 16:12".
    public internal(set) var savedAt: Date?

    /// It has gone to the Outbox, or been thrown away: nothing left to save.
    func markSent() { sent = true }

    // MARK: the frame (T079; screens 05 and 06)

    /// What the title says this is: "New message", "Reply to all".
    public var heading: String { composerTitle(kind: draft.kind) }

    /// "Draft saved locally 16:12", once a save has landed; nothing before.
    public var savedWords: String? {
        savedAt.map { draftSavedWords(at: Int64($0.timeIntervalSince1970 * 1000)) }
    }

    /// What will be sent, counted: "Plain text · 58 words".
    public var summary: String { draftSummary(draft: edited) }

    /// "Remind if no reply": when, in epoch milliseconds, or `nil` for none.
    /// Saved and sent with the draft.
    public var remindAt: Int64?

    /// The footer's verb: "Remind if no reply", and its day once one is
    /// chosen ("Remind if no reply · Tue 29 Sep").
    public var remindWords: String { remindMeaning(at: remindAt) }

    /// The quote a reply opened with, folded under what is written (screen
    /// 06): `body` holds only what is written above it, and `edited` puts
    /// it back, so what is saved and sent still quotes. `nil` once shown,
    /// and for a draft that ends in no quote.
    public private(set) var quoteFold: QuoteFoldFfi?

    /// Unfold the quote into the body, where it can be edited.
    public func showQuote() {
        guard let fold = quoteFold else { return }
        body += fold.quote
        quoteFold = nil
    }

    /// The account it is written from, and its address as the From line
    /// says it.
    public private(set) var account: Int64
    public private(set) var from: String

    /// The From picker: write from `account` instead.
    public func choose(account: AccountFfi) {
        self.account = account.id
        from = account.displayName.isEmpty
            ? account.address : "\(account.displayName) <\(account.address)>"
    }

    // MARK: recipient completion

    /// What completes the words in `suggesting`, best first; empty until a
    /// recipient is typed (screen 06: no contact list until then).
    public private(set) var suggestions: [RecipientSuggestionFfi] = []
    /// The recipient field the list hangs under.
    public private(set) var suggesting: RecipientField?
    /// The row ↑↓ are on.
    public private(set) var highlighted = 0

    /// The text of `field` now.
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

    /// The engine's answer for `text` in `field`: shown while the field
    /// still holds those words, dropped otherwise.
    public func suggest(_ answer: [RecipientSuggestionFfi], in field: RecipientField, for text: String) {
        guard self.text(of: field) == text else { return }
        suggestions = answer
        suggesting = answer.isEmpty ? nil : field
        highlighted = 0
    }

    /// ↑ or ↓ in the list.
    public func moveSuggestion(by step: Int) {
        guard !suggestions.isEmpty else { return }
        highlighted = min(max(highlighted + step, 0), suggestions.count - 1)
    }

    /// Return or Tab: the highlighted suggestion becomes the field's whole
    /// text, as the engine wrote it. `false` with nothing to accept.
    @discardableResult
    public func acceptSuggestion() -> Bool {
        guard let field = suggesting, suggestions.indices.contains(highlighted) else { return false }
        set(field, to: suggestions[highlighted].accepted)
        dismissSuggestions()
        return true
    }

    /// Esc, or the field left: the list goes, the words stay.
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
        // Shown from the start when they hold somebody. A recipient that is
        // not on screen is a recipient the writer cannot remove, and
        // `reply_all` puts every other original recipient in `cc` — so `E`
        // opened a window showing one address and silently addressing
        // everyone else.
        showsCopyFields = !draft.cc.isEmpty || !draft.bcc.isEmpty
        subject = draft.subject
        bodyHtml = draft.bodyHtml
        rich = draft.rich
        account = draft.account
        from = draft.from
        remindAt = draft.remindAt
        // A plain reply's quote folds under what is written (screen 06). A
        // rich draft keeps its document whole: the editing surface is the
        // one `editor.js` shared with GTK, and is not cut here.
        if !draft.rich, let fold = foldQuote(body: draft.body) {
            body = fold.written
            quoteFold = fold
        } else {
            body = draft.body
        }
    }

    /// The window's title: the subject, or what an unnamed draft is called.
    ///
    /// The canvas puts the subject in the title bar, which is what makes two
    /// compose windows tellable apart in the Window menu — the one place a
    /// window's title is load-bearing on this platform.
    public var title: String {
        let trimmed = subject.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "New message" : trimmed
    }

    /// The footer: where the draft lives, and what will be sent.
    ///
    /// Both halves are claims Postio should be willing to make on screen, so
    /// both have to be *true*. The path is abbreviated the way a person
    /// writes it — `~/Library/…` — because a footer is read at a glance and
    /// `/Users/diego` is a prefix that tells them nothing they do not know.
    ///
    /// The MIME shape is the boundary's wording and follows what will
    /// actually leave, which is why `rich` is not simply the switch: rich
    /// composition is not built (#1271), so a message sent from here is
    /// plain however the switch is drawn.
    public var footer: String {
        "draft in \(PostioPath.abbreviated(draft.path)) · \(outgoingShape(rich: sendsRich))"
    }

    /// How many recipients this message has, and on which field.
    ///
    /// `nil` until there is more than one person on it: a banner that is
    /// always there is a banner nobody reads.
    ///
    /// The counting is the boundary's, so both composers say the same thing.
    /// FR-023 exists because **a reply-to-all to a large list looks exactly
    /// like a reply until it is sent** — and on this platform it looked even
    /// more like one, because until the Cc and Bcc fields arrived there was
    /// nothing on screen naming the crowd at all.
    public var recipientSummary: String? { PostioFFI.recipientSummary(draft: edited) }

    /// Whether this draft will actually leave as rich mail.
    ///
    /// It *is* the switch now (#1271). It was hardcoded `false` while the
    /// body was a text field, because the footer is a claim about what goes
    /// on the wire and there was no rich document to put there. There is
    /// one, so the claim can follow the control again.
    public var sendsRich: Bool { rich }

    /// Whether the format bar's marks do anything.
    ///
    /// The marks were drawn permanently disabled, which made the whole bar
    /// decoration. What decides now is the switch -- and being handed off to
    /// another editor, which is the one case where nothing in this window
    /// may write to the body at all.
    public var marksApply: Bool { rich && !isHandedOff }

    /// The marks in force where the caret sits, as registry command ids.
    ///
    /// Reported by the editing bridge on selection changes and edits, so a
    /// toolbar toggle can reflect the document rather than guess at it. The
    /// ids are the registry's, which is what lets the bar be built from
    /// `ComposeFormat.marks` and still light up correctly.
    public var caretMarks: Set<String> = []

    /// Whether the caret is inside `command`'s mark.
    public func isMarkActive(_ command: String) -> Bool { caretMarks.contains(command) }

    /// One press of a format button, waiting for the surface to apply it.
    ///
    /// The button lives in SwiftUI and the document lives in an
    /// `NSViewRepresentable`, so the press has to be left somewhere the
    /// surface will see on its next update rather than called straight
    /// through.
    public struct MarkRequest: Equatable, Sendable {
        /// The registry command — `bold`, `quote_block`.
        public let command: String
        /// Which press this is.
        ///
        /// Bold is a toggle, so pressing it twice has to reach the document
        /// twice. A request keyed only on the command would look unchanged
        /// the second time and the second press would be swallowed.
        public let serial: Int
        /// For `insert_link`: where it points. `nil` for every other mark.
        public let href: String?
    }

    /// The most recent press, or `nil` before there has been one.
    public private(set) var markRequest: MarkRequest?
    private var marksAsked = 0

    /// Whether a link is being asked for.
    ///
    /// A request rather than a call: the address comes from a sheet, and a
    /// command cannot put one up. The view watches this, asks, and calls
    /// `applyMark("insert_link", href:)` with the answer.
    public var wantsLink = false

    /// Whether the person has asked to throw this draft away.
    ///
    /// `Recovery::Confirm` in the registry, so the *view* has a dialog to
    /// show — a command that discarded on the spot would be a destructive
    /// verb with no way back, which is precisely what the registry says this
    /// one is not.
    public var wantsDiscard = false

    /// Whether an attachment is being asked for.
    ///
    /// Same shape as `wantsLink`: the file comes from an open panel, which a
    /// command cannot present.
    public var wantsAttachment = false

    /// Whether a picture for the body is being asked for (#1571).
    ///
    /// Same shape as `wantsAttachment`: the file comes from an open panel,
    /// which a command cannot present. Set through `askForImage`, which is
    /// where a draft that cannot take one says so instead.
    public var wantsImage = false

    /// A picture the boundary has stored, waiting for the surface to draw it.
    ///
    /// The part is already in the store and on the draft by the time this
    /// exists; what is left is running `script` in the document, once.
    public struct ImageRequest: Equatable, Sendable {
        /// `postio_ui::compose::image_script` over the part just minted.
        public let script: String
        /// Which insertion this is, so the same picture twice is two.
        public let serial: Int
    }

    /// The most recent picture, or `nil` before there has been one.
    public private(set) var imageRequest: ImageRequest?
    private var imagesAsked = 0

    /// Whether the reminder's times are being asked for (⌘H): a command
    /// cannot put a list up, so the view does.
    public var wantsRemind = false

    /// Whether the schedule-send picker is being asked for.
    ///
    /// The four times it offers are the boundary's — `schedulePresets()` —
    /// so a Mac and a Linux desktop mean the same thing by "tomorrow
    /// morning".
    public var wantsSchedule = false

    /// Ask the surface to apply `command` to the selection.
    ///
    /// Ignored on a plain draft: the bar is disabled there, but the keyboard
    /// reaches this too, and `⌘B` over a document that cannot carry marks
    /// has nothing to apply to.
    public func applyMark(_ command: String, href: String? = nil) {
        guard marksApply else { return }
        marksAsked += 1
        markRequest = MarkRequest(command: command, serial: marksAsked, href: href)
    }

    /// The Rich/Plain switch has gone to Plain; `text` is what the document
    /// reads as.
    ///
    /// The composer's rule is "whichever surface is active is
    /// authoritative", and it has nothing to say about the moment of the
    /// switch — which is exactly when the field about to become
    /// authoritative is the stale one. Everything typed in Rich went into
    /// the document, so `body` was never touched; without this, switching to
    /// Plain and sending queued an empty message (#1293).
    ///
    /// The marks are kept, because the switch is on the document: turning it
    /// back on must cost nothing.
    ///
    /// Words already in the plain field are not overwritten. Somebody who
    /// wrote plain, tried Rich and came back has text there that is theirs.
    public func switchedToPlain(text: String) {
        rich = false
        if body.isEmpty {
            body = text
        }
    }

    /// Ask for a picture to put in the body, or say why this draft cannot
    /// take one.
    ///
    /// A plain draft has no document to hold a picture, and a handed-off one
    /// may not be written from this window at all. Both are said rather than
    /// swallowed: `insert_image` doing nothing is indistinguishable from
    /// `insert_image` being broken.
    public func askForImage() {
        guard marksApply else {
            status = isHandedOff
                ? "This draft is open in another editor, so nothing can be added to its body here."
                : "A picture goes in the body of a rich message. Turn on Rich to put one there."
            return
        }
        wantsImage = true
    }

    /// Put `bytes` in the body as a picture.
    ///
    /// The boundary stores the part and saves the draft before answering, so
    /// the picture survives the window closing the way an attachment does.
    public func insertImage(_ bytes: Data, mimeType: String, through session: PostioSession?) {
        guard let session, marksApply else { return }
        do {
            took(try session.insertImage(bytes, mimeType: mimeType, into: edited))
        } catch {
            status = error.localizedDescription
        }
    }

    /// Put the picture in `file` in the body.
    ///
    /// Its type is read from its bytes, and falls back to its name only when
    /// the bytes say nothing — in which case the boundary refuses it in its
    /// own words, pointing at Attach file.
    public func insertImage(from file: URL, through session: PostioSession?) {
        let bytes: Data
        do {
            bytes = try Data(contentsOf: file)
        } catch {
            // The name is not in the sentence: it is the user's, and a status
            // line is a thing people screenshot.
            status = "That picture could not be read."
            return
        }
        insertImage(bytes, mimeType: MimeType.ofImage(bytes) ?? MimeType.of(file), through: session)
    }

    /// Take a picture the boundary stored: the draft with its part on it, and
    /// the script that draws it at the caret.
    public func took(_ inserted: InlineImageFfi) {
        draft = inserted.draft
        syncFromDraft()
        imagesAsked += 1
        imageRequest = ImageRequest(script: inserted.script, serial: imagesAsked)
        status = nil
    }

    /// Say why a link was refused, or clear the last complaint.
    ///
    /// A refused scheme is something to say rather than a link that is
    /// created, looks right, and vanishes at the next parse.
    public func refuseLink(_ href: String) {
        status = "A message can only link to http, https or mailto — not \(href)."
    }

    /// Take what a paste became, and say what it cost.
    ///
    /// The sentence is `postio_body::Lost::summary`'s, arriving through the
    /// boundary, so both composers say the same thing about the same paste.
    /// `nil` says nothing at all, which is the right answer when nothing was
    /// lost: a composer that announced every paste would train people to
    /// ignore the one that mattered.
    public func tookPaste(_ pasted: PastedFfi) {
        // Only the sentence. The paste is inserted *at the caret* by the
        // surface, and the bridge reports the resulting document back on the
        // `input` event it raises -- so writing the body here would replace
        // everything already typed with whatever was on the clipboard.
        status = pasted.dropped
    }


    /// The draft as the store should have it.
    public var edited: DraftFfi {
        var edited = draft
        edited.to = to
        edited.cc = cc
        edited.bcc = bcc
        edited.subject = subject
        edited.body = body + (quoteFold?.quote ?? "")
        edited.bodyHtml = bodyHtml
        edited.rich = rich
        edited.account = account
        edited.from = from
        edited.remindAt = remindAt
        return edited
    }

    /// Whether the Cc and Bcc rows are on screen.
    ///
    /// Hidden by default, because most mail has neither and two empty rows
    /// above every message is two rows of furniture. Shown the moment either
    /// holds an address, and — see [`toggleCopyFields`](Self.toggleCopyFields)
    /// — they will not hide again while one does.
    public private(set) var showsCopyFields: Bool = false

    /// Ask for the copy fields, or put them away.
    ///
    /// **They refuse to hide while they hold addresses.** That is GTK's rule
    /// and it is the whole safeguard: hiding a field that holds somebody is
    /// how a recipient becomes invisible, which is the state this pair of
    /// fields exists to end.
    public func toggleCopyFields() {
        if showsCopyFields && (!cc.isEmpty || !bcc.isEmpty) { return }
        showsCopyFields.toggle()
    }

    /// Throw this draft away.
    ///
    /// `sent` rather than a state of its own: both mean the window has
    /// nothing left to write and must not autosave on its way out — which
    /// `onDisappear` would otherwise do, putting the discarded draft straight
    /// back.
    public func discard(through session: PostioSession) {
        if id != 0 || draft.id != 0 {
            session.discardDraft(draft.id)
        }
        sent = true
        status = nil
    }

    /// Whether anything has been typed that the store does not have.
    public var isDirty: Bool { edited != draft }

    /// Write what has been typed, and remember the id the store assigned.
    ///
    /// The id matters more than it looks: `save` is idempotent on it, and a
    /// composer that forgot it would insert a new row on every autosave and
    /// fill the Drafts folder with one half-written message.
    public func save(through session: PostioSession) {
        _ = save { session.saveDraft($0) }
    }

    /// What the status line, and the toast, say when a save failed.
    public static let notSaved = "This draft could not be saved."

    /// Write what has been typed through `write`, and keep the id it
    /// answers with; `false` when it could not be written.
    @discardableResult
    public func save(with write: (DraftFfi) -> DraftFfi?) -> Bool {
        guard let saved = write(edited) else {
            status = Self.notSaved
            return false
        }
        // Only what the store owns: what is being typed stays as typed.
        draft.id = saved.id
        draft.attachments = saved.attachments
        draft.path = saved.path
        // Everything else is what was just written.
        draft.to = saved.to
        draft.cc = saved.cc
        draft.bcc = saved.bcc
        draft.subject = saved.subject
        draft.body = saved.body
        draft.bodyHtml = saved.bodyHtml
        draft.rich = saved.rich
        draft.remindAt = saved.remindAt
        draft.account = saved.account
        draft.from = saved.from
        status = nil
        return true
    }

    /// Attach files chosen in an open panel.
    ///
    /// Each is stored as it is taken, and the draft is saved on the way — so
    /// an attachment survives the window closing, which is the whole reason
    /// the bytes go to the store rather than a path being remembered.
    ///
    /// A file that will not attach stops that file and not the others: three
    /// dragged in with one unreadable should leave two attached and say which
    /// one did not.
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

    /// Queue this draft to leave at `when` — *Schedule send…*.
    ///
    /// The same shape as [`send`](Self::send): the window closes on the
    /// keystroke and the queue does the rest. Every refusal `send` makes is
    /// made here too, by the boundary, because being refused at 8am tomorrow
    /// — when nobody is watching this window — is strictly worse than being
    /// refused now.
    public func send(at when: Int64, through session: PostioSession?) {
        guard let session else { return }
        if let complaint = session.sendDraftLater(edited, at: when) {
            status = complaint
            return
        }
        status = nil
        sent = true
    }

    /// Take an attachment off again.
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

    /// What is attached, for the window to list.
    public var attachments: [AttachmentFfi] { draft.attachments }

    /// Take the fields back off the draft the store just answered with.
    ///
    /// Only the ones the store owns: the id it assigned and what is attached.
    /// Anything being typed stays as typed — a save that overwrote the
    /// subject somebody was halfway through is a save nobody would forgive.
    private func syncFromDraft() {}

    /// Where this draft is while another editor has it, or `nil`.
    public private(set) var handedOffTo: String?

    /// Whether the window should be read-only: another editor holds the
    /// draft, and two writers would each silently undo the other.
    public var isHandedOff: Bool { handedOffTo != nil }

    /// Which editor the hand-off will use — `[compose] editor` (#1288).
    ///
    /// Empty means the platform's own default. Held here rather than read on
    /// every render because the button is labelled from it; refreshed when
    /// the window appears, which is when it can have changed.
    public var editor = ""

    /// Re-read the configured editor. The compose window does this on appear.
    public func refreshEditor() {
        editor = ComposeHandoff.configuredEditor()
    }

    /// Hand the draft to another editor.
    ///
    /// The draft is saved first — an editor opened on a body Postio has not
    /// written down is one crash away from having been the only copy — and
    /// the window goes read-only until it comes back.
    ///
    /// It opens in the editor named by `[compose] editor`, or — with nothing
    /// chosen — in whatever this Mac opens a text file with. **Not
    /// `$EDITOR`**: an application launched from Finder has no shell
    /// environment, so `$EDITOR` is usually simply absent, and a button that
    /// silently did nothing for most people would be worse than one that is
    /// honest about which editor it means (#1288).
    ///
    /// `open` answers `nil` when the draft went somewhere and a sentence when
    /// it did not — including the case worth having a setting for at all, an
    /// editor that wants a terminal. A refusal takes the draft straight back,
    /// so the window is never left read-only waiting for an editor that never
    /// opened.
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

    /// Take the draft back from the other editor.
    ///
    /// Called when the window returns to the front, which is the moment a
    /// person means by "I am done there".
    public func takeBack(through session: PostioSession?) {
        guard let session, let path = handedOffTo else { return }
        do {
            draft = try session.endHandoff(of: edited, at: path)
            // The other editor had the whole body, quote and all.
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

    /// Queue it for sending. `true` when the window may close.
    ///
    /// Nothing here waits for a server: the send is a local write that
    /// `postio-sync` drains when there is a network. A refusal — no
    /// recipients, already queued — is the one case where the window must
    /// stay open, because the words are in it.
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
