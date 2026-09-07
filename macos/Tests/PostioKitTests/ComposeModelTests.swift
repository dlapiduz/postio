import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// Writing a message (#1272, canvas screen 26).
@MainActor
@Suite struct ComposeModelTests {
    private func draft(subject: String = "", rich: Bool = false) -> DraftFfi {
        DraftFfi(
            id: 0,
            account: 1,
            kind: .new,
            from: "Mara Ostwald <mara@example.com>",
            to: "",
            cc: "",
            bcc: "",
            subject: subject,
            body: "",
            rich: rich,
            inReplyTo: nil,
            path: "/Users/someone/mail",
            attachments: []
        )
    }

    @Test func anUnnamedDraftIsStillCalledSomething() {
        // The canvas puts the subject in the title bar, and the Window menu
        // is where two compose windows are told apart. "" is not a name.
        let model = ComposeModel(id: 1, draft: draft())
        #expect(model.title == "New message")

        model.subject = "  Re: the gate  "
        #expect(model.title == "Re: the gate", "trimmed, because a title bar is not a text field")
    }

    @Test func theFooterNamesTheFileAndWhatWillLeave() {
        let model = ComposeModel(id: 1, draft: draft())
        #expect(model.footer == "draft in /Users/someone/mail · text/plain, format=flowed")
    }

    @Test func theFooterSaysWhatWillActuallyLeave_notWhatTheSwitchSays() {
        // The switch is a control; the footer is a claim about the wire.
        // Rich composition is not built (#1271), so a message from here is
        // plain however the switch is drawn — and the footer must not say
        // otherwise, because that is the one thing it exists to say.
        let model = ComposeModel(id: 1, draft: draft(rich: true))
        model.rich = true

        #expect(model.footer.hasSuffix("text/plain, format=flowed"))
        #expect(!model.sendsRich)
    }

    @Test func theFooterWritesTheHomeDirectoryTheWayAPersonDoes() {
        // A footer is read at a glance, and `/Users/diego` is a prefix that
        // tells somebody nothing they do not already know.
        let path = NSHomeDirectory() + "/Library/Application Support/Postio/postio.db"
        #expect(PostioPath.abbreviated(path).hasPrefix("~/Library/"))
        #expect(PostioPath.abbreviated("/var/tmp/elsewhere") == "/var/tmp/elsewhere")
    }

    @Test func typingMakesTheDraftDirtyAndTheEditCarriesIt() {
        let model = ComposeModel(id: 1, draft: draft())
        #expect(!model.isDirty)

        model.to = "bo@example.com"
        #expect(model.isDirty)
        #expect(model.edited.to == "bo@example.com")
        #expect(model.edited.id == 0, "and it is still the same draft, unsaved")
    }

    @Test func switchingToPlainKeepsTheWords() {
        // The switch is on the document, not on the window: it changes what
        // will be built, not what has been written.
        let model = ComposeModel(id: 1, draft: draft(rich: true))
        model.body = "Six is fine."

        model.rich = false

        #expect(model.body == "Six is fine.")
        #expect(!model.edited.rich)
    }

    @Test func aStoreHandsOutOneWindowPerDraft() {
        let store = ComposeStore()
        store.open(draft(subject: "First"))
        let first = store.requested
        store.open(draft(subject: "Second"))

        #expect(store.count == 2, "several compose windows at once, as the canvas says")
        #expect(first != store.requested, "each is its own window")
        #expect(store.model(store.requested!)?.title == "Second")
    }

    @Test func twoRequestsForAWindowAreTwoRequests() {
        // `onChange` compares values, so a second `⌘N` must not look like
        // nothing happened — the same trap `⌘,` fell into (#1261).
        let store = ComposeStore()
        store.open(draft())
        let first = store.request
        store.open(draft())

        #expect(store.request != first)
    }

    @Test func closingAWindowForgetsItsDraft() {
        let store = ComposeStore()
        store.open(draft())
        let id = store.requested!

        store.close(id)

        #expect(store.count == 0)
        #expect(store.model(id) == nil)
    }

    @Test func attachingWithNoSessionChangesNothing() {
        // The window cannot outlive its session, but the model can be asked
        // anyway, and a crash is not an answer.
        let model = ComposeModel(id: 1, draft: draft())
        model.attach([URL(fileURLWithPath: "/tmp/gate.pdf")], through: nil)

        #expect(model.attachments.isEmpty)
        #expect(model.status == nil)
    }

    @Test func aDraftKnowsWhatIsAttachedToIt() {
        var carrying = draft()
        carrying.attachments = [
            AttachmentFfi(id: 7, filename: "gate-plan.pdf", mimeType: "application/pdf", size: "1.8 MB")
        ]
        let model = ComposeModel(id: 1, draft: carrying)

        #expect(model.attachments.count == 1)
        #expect(model.attachments[0].filename == "gate-plan.pdf")
        // And the edit carries it, so saving does not drop the file.
        #expect(model.edited.attachments.count == 1)
    }

    @Test func aFileTypeNothingRecognisesStillGetsAName() {
        // "Some bytes" beats refusing to attach a file because the type
        // database had never heard of it.
        #expect(MimeType.of(URL(fileURLWithPath: "/tmp/thing.qqq")) == MimeType.fallback)
        #expect(MimeType.of(URL(fileURLWithPath: "/tmp/notes.txt")) == "text/plain")
    }

    @Test func handingOffToAnEditorSavesFirstAndThenSaysItCannot() {
        let model = ComposeModel(id: 1, draft: draft())
        model.handOff()

        #expect(model.status?.contains("$EDITOR") == true)
        #expect(model.status?.contains("saved") == true)
    }
}
