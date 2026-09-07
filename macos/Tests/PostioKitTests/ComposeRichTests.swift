import PostioFFI
import Testing

@testable import PostioKit

/// Rich composition on macOS (#1271).
///
/// The window had the switch and the format bar and could not carry a mark:
/// the body was a `TextEditor`, so `rich` was a claim the composer could not
/// keep and its footer said "plain" whatever the switch was set to.
///
/// What is asserted here is the model, not the web view. The editing surface
/// itself needs a window server, which these suites deliberately do not
/// require (`WindowServerRequiredTests` says why a skip that looks like a
/// pass is worse than no test) — so the seam is: what the model hands the
/// surface, what it takes back, and what it then claims about the message.
@MainActor
@Suite struct ComposeRichTests {
    private func draft(rich: Bool = false, html: String? = nil, body: String = "") -> DraftFfi {
        DraftFfi(
            id: 4,
            account: 1,
            kind: .new,
            from: "Ada <ada@example.com>",
            to: "bo@example.com",
            cc: "",
            bcc: "",
            subject: "The gate",
            body: body,
            bodyHtml: html,
            rich: rich,
            inReplyTo: nil,
            path: "/tmp/postio",
            attachments: []
        )
    }

    @Test func aRichDraftOpensWithItsMarksRatherThanItsPlainText() {
        // Reopening a rich draft as plain text would silently discard every
        // mark the moment it was saved again.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>Six is <strong>fine</strong>.</p>"))
        #expect(model.rich)
        #expect(model.bodyHtml == "<p>Six is <strong>fine</strong>.</p>")
    }

    @Test func whatIsEditedIsCarriedBackAsTheRichHalf() {
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.bodyHtml = "<p>a<strong>b</strong></p>"
        #expect(model.edited.bodyHtml == "<p>a<strong>b</strong></p>")
        #expect(model.edited.rich)
    }

    @Test func theFooterFinallyTellsTheTruthAboutARichMessage() {
        // `sendsRich` was hardcoded false, because there was no rich document
        // to send. Now the switch is the answer, and the footer follows it.
        let rich = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(rich.sendsRich)
        #expect(rich.footer.contains("html"))

        let plain = ComposeModel(id: 4, draft: draft(rich: false))
        #expect(!plain.sendsRich)
        #expect(plain.footer.contains("format=flowed"))
    }

    @Test func turningTheSwitchOffKeepsTheMarksInCaseItGoesBackOn() {
        // The switch is on the document, not on the window. Discarding the
        // marks here would make a mis-click an unrecoverable loss with no
        // warning.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a<em>b</em></p>"))
        model.rich = false
        #expect(model.edited.bodyHtml == "<p>a<em>b</em></p>")
        #expect(!model.edited.rich)

        model.rich = true
        #expect(model.edited.bodyHtml == "<p>a<em>b</em></p>")
    }

    @Test func aDirtyRichBodyIsNoticedSoItGetsSaved() {
        // `isDirty` drives the autosave. A model that compared only the plain
        // text would let every mark be lost on close, which is the failure
        // that costs somebody a message.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(!model.isDirty)
        model.bodyHtml = "<p>a<strong>b</strong></p>"
        #expect(model.isDirty)
    }

    @Test func theFormatBarIsLiveOnARichDraftAndInertOnAPlainOne() {
        // The marks were drawn permanently disabled. What decides now is the
        // switch, which is a thing a person can act on.
        let rich = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(rich.marksApply)

        let plain = ComposeModel(id: 4, draft: draft(rich: false))
        #expect(!plain.marksApply)
    }

    @Test func pressingAMarkAsksTheSurfaceToApplyIt() {
        // The gap #1271 reported: the buttons existed and reached nothing.
        // A request rather than a direct call, because the surface is an
        // NSViewRepresentable and the button is in SwiftUI -- what is
        // asserted is that pressing one leaves something for the surface to
        // pick up.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(model.markRequest == nil)

        model.applyMark("bold")
        #expect(model.markRequest?.command == "bold")
    }

    @Test func pressingTheSameMarkTwiceIsTwoRequestsNotOne() {
        // Bold is a toggle: pressing it twice must reach the document twice.
        // A request keyed only on the command would look unchanged the
        // second time and the surface would never be told.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.applyMark("bold")
        let first = model.markRequest
        model.applyMark("bold")

        #expect(model.markRequest?.command == "bold")
        #expect(model.markRequest != first, "the second press was swallowed")
    }

    @Test func aMarkOnAPlainDraftIsNotEvenRequested() {
        // The bar is disabled there, but the model is what the keyboard
        // reaches too, and `b` over a plain document has nothing to apply to.
        let model = ComposeModel(id: 4, draft: draft(rich: false))
        model.applyMark("bold")
        #expect(model.markRequest == nil)
    }

    @Test func aPasteThatLostSomethingSaysSoOnTheWindow() {
        // The acceptance line. The sentence is the engine's, so both
        // composers say the same thing about the same paste; what is checked
        // here is that the window actually surfaces it rather than dropping
        // it on the floor.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.tookPaste(
            PastedFfi(html: "<p>hi</p>", text: "hi", dropped: "Pasted without 1 table."))
        #expect(model.status == "Pasted without 1 table.")
    }

    @Test func aPasteThatLostNothingSaysNothing() {
        // Silence is the right answer. A composer that announced every paste
        // would train people to ignore the one that mattered.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.tookPaste(PastedFfi(html: "<p>hi</p>", text: "hi", dropped: nil))
        #expect(model.status == nil)
    }

    @Test func linkIsTheOneMarkThatCarriesAnAddress() {
        // It cannot be a plain toggle: `markScript` has no answer for
        // `insert_link` on purpose, because a link needs somewhere to point.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.applyMark(ComposeFormat.link, href: "https://example.com")

        #expect(model.markRequest?.command == ComposeFormat.link)
        #expect(model.markRequest?.href == "https://example.com")
    }

    @Test func theOtherMarksCarryNoAddress() {
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.applyMark("bold")
        #expect(model.markRequest?.href == nil)
    }

    @Test func aRefusedLinkIsSaidOutLoudRatherThanDisappearingLater() {
        // Created, looks right, vanishes at the next parse is the failure
        // this avoids: the subset is http, https and mailto, and anything
        // else is refused where there is somebody to tell.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.refuseLink("javascript:alert(1)")
        #expect(model.status?.contains("javascript:alert(1)") == true)
        #expect(model.status?.contains("http") == true)
    }

    @Test func theLinkCommandNamedInTheBarIsOneTheRegistryHas() {
        // A literal that no longer matches the registry is a button that
        // silently does nothing -- which is what this whole issue is about.
        #expect(ComposeFormat.marks.contains { $0.command == ComposeFormat.link })
    }
}
