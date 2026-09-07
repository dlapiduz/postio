import Foundation
import PostioFFI

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

    public init(id: Int64, draft: DraftFfi) {
        self.id = id
        self.draft = draft
        to = draft.to
        cc = draft.cc
        subject = draft.subject
        body = draft.body
        bodyHtml = draft.bodyHtml
        rich = draft.rich
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
        edited.subject = subject
        edited.body = body
        edited.bodyHtml = bodyHtml
        edited.rich = rich
        return edited
    }

    /// Whether anything has been typed that the store does not have.
    public var isDirty: Bool { edited != draft }

    /// Write what has been typed, and remember the id the store assigned.
    ///
    /// The id matters more than it looks: `save` is idempotent on it, and a
    /// composer that forgot it would insert a new row on every autosave and
    /// fill the Drafts folder with one half-written message.
    public func save(through session: PostioSession) {
        guard let saved = session.saveDraft(edited) else {
            status = "This draft could not be saved."
            return
        }
        draft = saved
        status = nil
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
            body = draft.body
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

/// The compose windows that are open.
///
/// A store rather than a window per view, because only a view can open a
/// window and a command can be run from anywhere: `⌘N` in the main window
/// asks for a compose window, and this is where the draft waits until one
/// exists to draw it.
@MainActor
@Observable
public final class ComposeStore {
    private var models: [Int64: ComposeModel] = [:]
    private var nextId: Int64 = 1

    /// The window most recently asked for, as a count that changes even when
    /// two requests are for the same thing — see `WindowRequest`.
    public private(set) var request = WindowRequest(id: "compose")
    /// Which draft that request is about.
    public private(set) var requested: Int64?

    public init() {}

    /// Take a draft and ask for a window to write it in.
    public func open(_ draft: DraftFfi) {
        let id = nextId
        nextId += 1
        models[id] = ComposeModel(id: id, draft: draft)
        requested = id
        request.raise()
    }

    /// The model a window is drawing, if it is still open.
    public func model(_ id: Int64) -> ComposeModel? { models[id] }

    /// Forget a window that has closed.
    public func close(_ id: Int64) { models[id] = nil }

    /// How many are open, for the tests and for anything that has to know
    /// whether closing the last one means anything.
    public var count: Int { models.count }
}
