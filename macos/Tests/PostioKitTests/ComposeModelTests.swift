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
            bodyHtml: nil,
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

    @Test func theFooterSaysWhatWillActuallyLeave() {
        // The switch is a control; the footer is a claim about the wire, and
        // the two agreeing is not automatic. This asserted the footer stayed
        // *plain* while the switch said Rich, because there was no rich
        // document to send (#1271) — the invariant was never "always plain",
        // it was "the footer follows what leaves". Rich composition exists
        // now, so the same invariant has the other answer.
        let rich = ComposeModel(id: 1, draft: draft(rich: true))
        #expect(rich.footer.hasSuffix("html + text/plain"))
        #expect(rich.sendsRich)

        // And the half that is easy to lose and expensive to notice: rich
        // still carries a plain alternative, so the footer names both.
        #expect(rich.footer.contains("text/plain"))

        let plain = ComposeModel(id: 1, draft: draft(rich: false))
        #expect(plain.footer.hasSuffix("text/plain, format=flowed"))
        #expect(!plain.sendsRich)
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

    @Test func nothingIsHandedOffToBeginWith() {
        let model = ComposeModel(id: 1, draft: draft())
        #expect(!model.isHandedOff)
    }

    @Test func handingOffWithNoSessionDoesNothingAndOpensNothing() async {
        // The window cannot outlive its session, but the model can be asked.
        var opened: [URL] = []
        let model = ComposeModel(id: 1, draft: draft())

        await model.handOff(through: nil) { opened.append($0); return nil }

        #expect(opened.isEmpty)
        #expect(!model.isHandedOff)
    }

    // -- which editor a draft goes to (#1288) -----------------------------

    @Test func nothingIsChosenUntilSomebodyChoosesIt() {
        // Empty is the shipped state, and it means "whatever this Mac opens
        // a text file with" rather than a named application that may not be
        // installed.
        let model = ComposeModel(id: 1, draft: draft())

        #expect(model.editor.isEmpty)
        #expect(settingsHandoffLabel(configured: model.editor) == "Edit elsewhere")
    }

    @Test func theButtonNamesTheEditorOnceOneIsChosen() {
        // `Open in $EDITOR` is canvas 26's label; an application launched
        // from Finder has no `$EDITOR`, so the button names the setting.
        let model = ComposeModel(id: 1, draft: draft())
        model.editor = "Some Editor"

        #expect(settingsHandoffLabel(configured: model.editor) == "Open in Some Editor")
    }

    @Test func aTerminalEditorIsNotOpenedAndSaysWhy() {
        // `vi` ships with macOS and has no window of its own.
        #expect(ComposeHandoff.found("vi") == .terminalProgram)

        let target = settingsHandoffTarget(configured: "vi", found: .terminalProgram)
        guard case .needsTerminal(_, let advice) = target else {
            Issue.record("expected a terminal program, got \(target)")
            return
        }
        #expect(advice.contains("terminal"))
    }

    @Test func anEditorThatIsNotThereSaysThatRatherThanBlamingTheTerminal() {
        // The distinction GTK's adoption forced (#1297): a name that is not
        // an application is a terminal program on macOS *usually* — and a
        // typo the rest of the time. Nothing on this Mac is called this.
        let typo = "postio-no-such-editor-1297"
        #expect(ComposeHandoff.found(typo) == .nothing)

        let target = settingsHandoffTarget(configured: typo, found: .nothing)
        guard case .missing(_, let advice) = target else {
            Issue.record("expected a missing editor, got \(target)")
            return
        }
        #expect(advice.contains(typo))
        #expect(
            !advice.contains("terminal"),
            "a misspelled editor is not a terminal one: \(advice)"
        )
    }

    @Test func anApplicationEveryMacHasIsFound() {
        // The other half, and the reason the case above means anything: a
        // lookup that never finds anything would make every editor look like
        // a terminal program. TextEdit ships with macOS, by name and by
        // bundle identifier both.
        #expect(ComposeHandoff.isApplication("TextEdit"))
        #expect(ComposeHandoff.isApplication("com.apple.TextEdit"))
        #expect(ComposeHandoff.isApplication("  TextEdit  "), "what somebody typed is trimmed")
    }

    @Test func aDraftIsHandedBackWhenTheEditorCouldNotTakeIt() async {
        // The window must never be left read-only waiting for an editor that
        // never opened.
        let model = ComposeModel(id: 1, draft: draft())

        await model.handOff(through: nil) { _ in "it would not open" }

        #expect(!model.isHandedOff)
    }

    @Test func takingItBackWithNothingOutIsHarmless() {
        // The window becoming active is what triggers this, and it becomes
        // active all the time.
        let model = ComposeModel(id: 1, draft: draft())
        model.takeBack(through: nil)

        #expect(!model.isHandedOff)
        #expect(model.status == nil)
    }
}
