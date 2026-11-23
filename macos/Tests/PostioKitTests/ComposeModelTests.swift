import Foundation
import PostioFFI
import Testing

@testable import PostioKit

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
        // The footer follows what leaves, not the switch.
        let rich = ComposeModel(id: 1, draft: draft(rich: true))
        #expect(rich.footer.hasSuffix("html + text/plain"))
        #expect(rich.sendsRich)

        #expect(rich.footer.contains("text/plain"))

        let plain = ComposeModel(id: 1, draft: draft(rich: false))
        #expect(plain.footer.hasSuffix("text/plain, format=flowed"))
        #expect(!plain.sendsRich)
    }

    @Test func theFooterWritesTheHomeDirectoryTheWayAPersonDoes() {
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
        let model = ComposeModel(id: 1, draft: draft(rich: true))
        model.body = "Six is fine."

        model.rich = false

        #expect(model.body == "Six is fine.")
        #expect(!model.edited.rich)
    }

    @Test func attachingWithNoSessionChangesNothing() {
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
        #expect(model.edited.attachments.count == 1)
    }

    @Test func aFileTypeNothingRecognisesStillGetsAName() {
        #expect(MimeType.of(URL(fileURLWithPath: "/tmp/thing.qqq")) == MimeType.fallback)
        #expect(MimeType.of(URL(fileURLWithPath: "/tmp/notes.txt")) == "text/plain")
    }

    @Test func nothingIsHandedOffToBeginWith() {
        let model = ComposeModel(id: 1, draft: draft())
        #expect(!model.isHandedOff)
    }

    @Test func handingOffWithNoSessionDoesNothingAndOpensNothing() async {
        var opened: [URL] = []
        let model = ComposeModel(id: 1, draft: draft())

        await model.handOff(through: nil) { opened.append($0); return nil }

        #expect(opened.isEmpty)
        #expect(!model.isHandedOff)
    }

    @Test func nothingIsChosenUntilSomebodyChoosesIt() {
        let model = ComposeModel(id: 1, draft: draft())

        #expect(model.editor.isEmpty)
        #expect(settingsHandoffLabel(configured: model.editor) == "Edit elsewhere")
    }

    @Test func theButtonNamesTheEditorOnceOneIsChosen() {
        let model = ComposeModel(id: 1, draft: draft())
        model.editor = "Some Editor"

        #expect(settingsHandoffLabel(configured: model.editor) == "Open in Some Editor")
    }

    @Test func aDraftIsHandedBackWhenTheEditorCouldNotTakeIt() async {
        // The window must never be left read-only waiting for an editor that never opened.
        let model = ComposeModel(id: 1, draft: draft())

        await model.handOff(through: nil) { _ in "it would not open" }

        #expect(!model.isHandedOff)
    }

    @Test func takingItBackWithNothingOutIsHarmless() {
        let model = ComposeModel(id: 1, draft: draft())
        model.takeBack(through: nil)

        #expect(!model.isHandedOff)
        #expect(model.status == nil)
    }

    private func replyAll() -> DraftFfi {
        var draft = self.draft(subject: "Re: Radon reduction")
        draft.to = "Ada Norwood <ada@example.com>"
        draft.cc = "Bo Ferris <bo@example.com>, Quinn Vale <quinn@example.net>"
        draft.bcc = "Archive <archive@example.invalid>"
        return draft
    }

    @Test func aBccTheWriterNeverSawIsStillSent() {
        // `edited` once copied the draft wholesale, so a `bcc` nothing on screen showed rode
        // through to the send; a `mailto:` link carries one.
        let model = ComposeModel(id: 1, draft: replyAll())
        #expect(model.bcc == "Archive <archive@example.invalid>")

        model.bcc = ""
        #expect(model.edited.bcc.isEmpty, "the writer removed it and it went anyway")
    }

    @Test func everyRecipientFieldSurvivesARoundTrip() {
        let model = ComposeModel(id: 1, draft: replyAll())
        #expect(model.edited.to == "Ada Norwood <ada@example.com>")
        #expect(model.edited.cc == "Bo Ferris <bo@example.com>, Quinn Vale <quinn@example.net>")
        #expect(model.edited.bcc == "Archive <archive@example.invalid>")
    }

    @Test func editingBccMakesTheDraftDirty() {
        // Autosave is driven by `isDirty`: a field it cannot see loses its edits.
        let model = ComposeModel(id: 1, draft: replyAll())
        #expect(!model.isDirty)
        model.bcc = "someone@example.test"
        #expect(model.isDirty)
    }

    @Test func theCopyFieldsShowThemselvesWhenTheyHoldAddresses() {
        let plain = ComposeModel(id: 1, draft: draft())
        #expect(!plain.showsCopyFields, "an empty Cc and Bcc stay out of the way")

        let all = ComposeModel(id: 1, draft: replyAll())
        #expect(all.showsCopyFields, "addresses nobody can see are addresses nobody can remove")
    }

    @Test func thePersonWritingCanAskForTheCopyFields() {
        let model = ComposeModel(id: 1, draft: draft())
        model.toggleCopyFields()
        #expect(model.showsCopyFields)
        model.toggleCopyFields()
        #expect(!model.showsCopyFields)
    }

    @Test func theCopyFieldsWillNotHideOverAnAddress() {
        let model = ComposeModel(id: 1, draft: replyAll())
        model.toggleCopyFields()
        #expect(model.showsCopyFields, "they hold addresses, so they stay")
    }

    @Test func aReplyAllSaysHowManyPeopleAreOnIt() {
        let model = ComposeModel(id: 1, draft: replyAll())
        #expect(model.recipientSummary == "1 To, 2 Cc, 1 Bcc")
    }

    @Test func anOrdinaryMessageSaysNothingAboutItsRecipients() {
        let model = ComposeModel(id: 1, draft: draft())
        model.to = "Ada Norwood <ada@example.com>"
        #expect(model.recipientSummary == nil)
    }

    @Test func theSummaryFollowsWhatIsTypedRatherThanWhatWasSaved() {
        // Reads `edited`, not `draft`: a banner counting a removed Cc would say something untrue.
        let model = ComposeModel(id: 1, draft: replyAll())
        model.cc = ""
        model.bcc = ""
        #expect(model.recipientSummary == nil)
    }

}
