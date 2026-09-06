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
    /// Both halves are claims Postio should be willing to make on screen. The
    /// MIME shape is the boundary's wording, not this frontend's.
    public var footer: String {
        "draft in \(draft.path) · \(outgoingShape(rich: rich))"
    }

    /// The draft as the store should have it.
    public var edited: DraftFfi {
        var edited = draft
        edited.to = to
        edited.cc = cc
        edited.subject = subject
        edited.body = body
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

    /// Attaching a file, which this build cannot do yet.
    ///
    /// Said out loud rather than silently ignored. An attachment has to reach
    /// the blob store through the boundary, and that path does not exist yet
    /// (#1269) — a paperclip that appeared to work and sent nothing would be
    /// worse than one that says so, because the sender would find out from
    /// the recipient.
    public func attach(_ files: [URL], through session: PostioSession?) {
        guard !files.isEmpty else { return }
        let names = files.map(\.lastPathComponent).joined(separator: ", ")
        status = "Attachments are not built yet, so \(names) was not added."
    }

    /// Handing the draft to `$EDITOR`, which this build cannot do yet.
    ///
    /// Same rule as `attach`: the draft is saved first, so nothing typed is
    /// at risk, and then the composer says what it cannot do (#1270).
    public func handOff() {
        status = "Editing in $EDITOR is not built yet. The draft is saved."
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
