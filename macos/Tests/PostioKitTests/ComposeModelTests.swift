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
            path: "/Users/someone/mail"
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

        model.rich = true
        #expect(
            model.footer.hasSuffix("html + text/plain"),
            "rich mail always carries a plain alternative, and the footer says so"
        )
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

    @Test func aThingTheComposerCannotDoYetSaysSoRatherThanSeemingToWork() {
        // A paperclip that appears to work and sends nothing is worse than
        // one that says so: the sender finds out from the recipient.
        let model = ComposeModel(id: 1, draft: draft())
        model.attach([URL(fileURLWithPath: "/tmp/gate.pdf")], through: nil)

        #expect(model.status?.contains("gate.pdf") == true)
        #expect(model.status?.contains("not built yet") == true)
    }

    @Test func handingOffToAnEditorSavesFirstAndThenSaysItCannot() {
        let model = ComposeModel(id: 1, draft: draft())
        model.handOff()

        #expect(model.status?.contains("$EDITOR") == true)
        #expect(model.status?.contains("saved") == true)
    }
}
