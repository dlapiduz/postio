import PostioFFI
import Testing
import WebKit

@testable import PostioAppKit
@testable import PostioKit

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
        // Reopening a rich draft as plain would discard every mark on the next save.
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
        let rich = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(rich.sendsRich)
        #expect(rich.footer.contains("html"))

        let plain = ComposeModel(id: 4, draft: draft(rich: false))
        #expect(!plain.sendsRich)
        #expect(plain.footer.contains("format=flowed"))
    }

    @Test func turningTheSwitchOffKeepsTheMarksInCaseItGoesBackOn() {
        // The switch is on the document, so discarding marks would make a mis-click unrecoverable.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a<em>b</em></p>"))
        model.rich = false
        #expect(model.edited.bodyHtml == "<p>a<em>b</em></p>")
        #expect(!model.edited.rich)

        model.rich = true
        #expect(model.edited.bodyHtml == "<p>a<em>b</em></p>")
    }

    @Test func aDirtyRichBodyIsNoticedSoItGetsSaved() {
        // A model comparing only plain text would let marks be lost on close.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(!model.isDirty)
        model.bodyHtml = "<p>a<strong>b</strong></p>"
        #expect(model.isDirty)
    }

    @Test func theFormatBarIsLiveOnARichDraftAndInertOnAPlainOne() {
        let rich = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(rich.marksApply)

        let plain = ComposeModel(id: 4, draft: draft(rich: false))
        #expect(!plain.marksApply)
    }

    @Test func pressingAMarkAsksTheSurfaceToApplyIt() {
        // A request, not a direct call: the surface is an `NSViewRepresentable` and the button is
        // SwiftUI.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        #expect(model.markRequest == nil)

        model.applyMark("bold")
        #expect(model.markRequest?.command == "bold")
    }

    @Test func pressingTheSameMarkTwiceIsTwoRequestsNotOne() {
        // A request keyed only on the command would look unchanged the second press.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.applyMark("bold")
        let first = model.markRequest
        model.applyMark("bold")

        #expect(model.markRequest?.command == "bold")
        #expect(model.markRequest != first, "the second press was swallowed")
    }

    @Test func aMarkOnAPlainDraftIsNotEvenRequested() {
        let model = ComposeModel(id: 4, draft: draft(rich: false))
        model.applyMark("bold")
        #expect(model.markRequest == nil)
    }

    @Test func aPasteThatLostSomethingSaysSoOnTheWindow() {
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.tookPaste(
            PastedFfi(html: "<p>hi</p>", text: "hi", dropped: "Pasted without 1 table."))
        #expect(model.status == "Pasted without 1 table.")
    }

    @Test func aPasteThatLostNothingSaysNothing() {
        // Silence is right: a composer announcing every paste trains people to ignore the one that
        // matters.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.tookPaste(PastedFfi(html: "<p>hi</p>", text: "hi", dropped: nil))
        #expect(model.status == nil)
    }

    @Test func linkIsTheOneMarkThatCarriesAnAddress() {
        // `markScript` has no answer for `insert_link` on purpose: a link needs somewhere to point.
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
        // The subset is http, https and mailto; anything else is refused where someone can be told.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a</p>"))
        model.refuseLink("javascript:alert(1)")
        #expect(model.status?.contains("javascript:alert(1)") == true)
        #expect(model.status?.contains("http") == true)
    }

    @Test func theLinkCommandNamedInTheBarIsOneTheRegistryHas() {
        #expect(ComposeFormat.marks.contains { $0.command == ComposeFormat.link })
    }

    @Test func switchingToPlainCarriesTheWordsIntoThePlainField() {
        // Everything is typed into the rich surface, so `body` is never touched; do not set it by
        // hand, which hid this bug.
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>The gate closes at six.</p>"))
        #expect(model.body.isEmpty)

        model.switchedToPlain(text: "The gate closes at six.")

        #expect(model.body == "The gate closes at six.")
        #expect(!model.rich)
        #expect(model.bodyHtml == "<p>The gate closes at six.</p>")
    }

    @Test func switchingToPlainDoesNotOverwriteWordsAlreadyTypedThere() {
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>marks</p>", body: "typed"))
        model.switchedToPlain(text: "marks")
        #expect(model.body == "typed")
    }

    @Test func theEditorAnswersEverySchemeItsShellNames() {
        // With no handler for `postio-editor:///` WebKit never started the load: Rich was an empty
        // box with nothing logged.
        let configuration = WKWebViewConfiguration()
        ComposeEditor.answerSchemes(
            in: configuration, cid: CidSchemeHandler { _, _ in nil })
        let base = URL(string: ComposeEditor.Coordinator.base)?.scheme
        #expect(base == "postio-editor")
        #expect(configuration.urlSchemeHandler(forURLScheme: base ?? "") != nil)
        #expect(configuration.urlSchemeHandler(forURLScheme: "postio-cid") != nil)
    }

    @Test func switchingToRichCarriesThePlainWordsIntoTheDocument() {
        // New drafts open plain; the surface was seeded from an empty document and the words
        // vanished.
        let model = ComposeModel(id: 4, draft: draft(body: "The gate closes at six."))

        model.switchedToRich(html: "<p>The gate closes at six.</p>")

        #expect(model.rich)
        #expect(model.bodyHtml == "<p>The gate closes at six.</p>")
    }

    @Test func goingBackToRichUntouchedKeepsTheMarks() {
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a<em>b</em></p>"))
        model.switchedToPlain(text: "ab")

        model.switchedToRich(html: "<p>ab</p>")

        #expect(model.bodyHtml == "<p>a<em>b</em></p>")
    }

    @Test func whatWasTypedInPlainIsWhatRichShowsNext() {
        let model = ComposeModel(id: 4, draft: draft(rich: true, html: "<p>a<em>b</em></p>"))
        model.switchedToPlain(text: "ab")
        model.body = "ab, and more"

        model.switchedToRich(html: "<p>ab, and more</p>")

        #expect(model.bodyHtml == "<p>ab, and more</p>")
    }

    @Test func whatWasTypedInRichIsWhatPlainShowsNext() {
        let model = ComposeModel(id: 4, draft: draft(body: "typed"))
        model.switchedToRich(html: "<p>typed</p>")
        model.bodyHtml = "<p>typed, then more</p>"

        model.switchedToPlain(text: "typed, then more")

        #expect(model.body == "typed, then more")
    }

    @Test func theDocumentRichStartsFromIncludesAFoldedQuote() {
        // A reply's quote is folded out of `body`; leaving it out of the document would drop it.
        let model = ComposeModel(
            id: 4, draft: draft(body: "Yes.\n\nOn Monday, Bo wrote:\n> Six?\n"))
        #expect(model.plainText == model.edited.body)
    }
}
