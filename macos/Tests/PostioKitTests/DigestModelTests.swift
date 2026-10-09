import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The digest's window, as the controller's intents leave it
/// (specs/009-focus-macos T114, screens 22 and 23).
///
/// The controller (`crates/postio-focus/src/digest.rs`) keeps the page, the
/// focused reference and row, which email is open in place, and what each
/// key does. The Mac opens the window on `FocusOpenDigest`, draws every
/// `FocusDigest` whole, reads the open email's header and body when
/// `view.email` names a new one, and hands back the pointer and a close
/// the toolkit made.
@MainActor
struct DigestModelTests {
    final class Engine: DigestEngine {
        var pointed: [UInt32] = []
        var referenced: [UInt32] = []
        var invoked: [String] = []
        var bindings: [String: String] = [:]

        func focusDigestPoint(_ index: UInt32) { pointed.append(index) }
        func focusDigestReference(_ index: UInt32) { referenced.append(index) }
        func invoke(_ id: String) { invoked.append(id) }
        func binding(for command: String) -> String? { bindings[command] }
    }

    typealias Source = MessageWindowModelTests.Source

    static func statement(_ index: UInt32, _ text: String, number: UInt32, message: Int64) -> DigestStatementFfi {
        DigestStatementFfi(index: index, text: text, number: number, message: message)
    }

    static func view(
        page: DigestPageFfi = .summary, focusedReference: UInt32? = 0, focused: UInt32? = 0,
        email: DigestEmailFfi? = nil, loading: Bool = false
    ) -> DigestViewFfi {
        DigestViewFfi(
            delivery: 5, page: page,
            title: email == nil ? "Weekly \u{b7} Newsletters" : "Issue 12: Quiet season",
            subtitle: email == nil ? "3 messages from 2 senders \u{b7} came due today 16:00" : "Source 2 of 3",
            archive: "Archive all 3", archiveKey: "A",
            ruleLine: "Weekly, Sunday 09:00 \u{b7} Edit rule and cadence", ruleKey: "d",
            tabs: true, listTab: "3 messages", tabKey: "Tab",
            rows: [
                DigestLineFfi(message: 11, sender: "The Ledger", subject: "Rates", preview: nil, time: "Mon"),
                DigestLineFfi(message: 12, sender: "Crate Notes", subject: "Issue 12: Quiet season", preview: "This week", time: "Tue"),
                DigestLineFfi(message: 13, sender: "Crate Notes", subject: "Issue 13", preview: nil, time: "Wed"),
            ],
            focused: focused,
            topics: [
                DigestTopicFfi(heading: "Rates \u{b7} 1 statement", statements: [
                    statement(0, "The committee held the rate.", number: 1, message: 11),
                ]),
                DigestTopicFfi(heading: "Engineering reading \u{b7} 2 statements", statements: [
                    statement(1, "Three libraries compared.", number: 2, message: 12),
                    statement(2, "A long read on collection.", number: 3, message: 13),
                ]),
            ],
            focusedReference: focusedReference,
            card: DigestCardFfi(title: "Reference 1 \u{b7} The Ledger \u{b7} Rates", hint: "open the full email", key: "Return"),
            footer: "Written on this computer from these 3 messages only.",
            email: email, back: "\u{2039} Summary", backKey: "Escape", loading: loading)
    }

    static func model() -> (DigestModel, Engine, Source) {
        let engine = Engine()
        let source = Source()
        return (DigestModel(engine: engine, source: source), engine, source)
    }

    @Test func aDigestRowOpensTheWindowAndItsViewDrawsTheSummary() {
        let (model, _, _) = Self.model()
        #expect(model.apply(.focusOpenDigest(delivery: 5)) == .open)
        #expect(model.isOpen)
        #expect(model.view == nil, "nothing is drawn before the view lands")

        #expect(model.apply(.focusDigest(view: Self.view())) == .redraw)
        #expect(model.view?.page == .summary)
        #expect(model.view?.topics.count == 2)
        #expect(model.focusedTopic == 0, "the card stands under the focused reference's topic")
        #expect(model.archiveCap == "⇧A")
        #expect(model.ruleCap == "d")
    }

    @Test func theFocusedReferenceMovesTheCardsTopic() {
        let (model, _, _) = Self.model()
        model.apply(.focusOpenDigest(delivery: 5))
        model.apply(.focusDigest(view: Self.view(focusedReference: 2)))
        #expect(model.focusedTopic == 1)
    }

    @Test func thePointerIsToldAndMovesNothingItself() {
        let (model, engine, _) = Self.model()
        model.apply(.focusOpenDigest(delivery: 5))
        model.apply(.focusDigest(view: Self.view()))
        model.reference(2)
        model.point(1)
        model.open(2)
        model.openReference()
        #expect(engine.referenced == [2])
        #expect(engine.pointed == [1, 2])
        #expect(engine.invoked == ["open_message", "open_message"])
        #expect(model.view?.focusedReference == 0, "moved only when the controller says")
    }

    @Test func theButtonsAreTheirCommands() {
        let (model, engine, _) = Self.model()
        model.apply(.focusOpenDigest(delivery: 5))
        model.apply(.focusDigest(view: Self.view()))
        model.archiveAll()
        model.editRule()
        model.toggleTab()
        model.back()
        #expect(engine.invoked == ["archive_thread", "digest_rule", "toggle_digest_summary", "back"])
    }

    @Test func anEmailOpenedInPlaceIsReadAndItsPassageIsTheHighlight() async {
        let (model, _, source) = Self.model()
        model.mainWidth = 1440
        model.apply(.focusOpenDigest(delivery: 5))
        let email = DigestEmailFfi(
            message: 12, number: 2, excerpt: "three libraries", banner: "Cited as 2 in the summary")
        #expect(model.apply(.focusDigest(view: Self.view(page: .email, focused: 1, email: email))) == .redraw)
        await model.settled()
        #expect(model.emailView?.message == 12)
        #expect(model.emailDocument != nil)
        #expect(source.views.first?.1 == 1 && source.views.first?.2 == 3, "its place among the digest's rows")
        #expect(source.documents.first?.2 == 1440)
        #expect(model.highlight == "three libraries")

        // The same email redrawn (a toast, a focus) is not read again.
        model.apply(.focusDigest(view: Self.view(page: .email, focused: 1, email: email)))
        await model.settled()
        #expect(source.views.count == 1)

        // Back to the summary forgets it.
        model.apply(.focusDigest(view: Self.view()))
        #expect(model.emailView == nil)
        #expect(model.emailDocument == nil)
    }

    @Test func itClosesOnItsOwnCloseAndAToolkitCloseIsSaidOnce() {
        let (model, _, _) = Self.model()
        model.apply(.focusOpenDigest(delivery: 5))
        model.apply(.focusDigest(view: Self.view()))
        #expect(model.apply(.focusCloseSurface(kind: .filtered)) == nil)
        #expect(model.apply(.focusCloseSurface(kind: .digest)) == .close)
        #expect(!model.isOpen)
        #expect(!model.closedByToolkit(), "the controller closed it: nothing to say")

        model.apply(.focusOpenDigest(delivery: 5))
        #expect(model.closedByToolkit(), "the close button: said once")
        #expect(!model.closedByToolkit())
    }

    @Test func aConfirmationIsAskedAndOnlyYesIsSaid() {
        let confirm = ConfirmFfi(
            token: 9, heading: "Stop digesting this sender?", body: "Their mail comes to the inbox.",
            confirm: "Stop digesting", destructive: true)
        let question = ConfirmQuestion(confirm)
        #expect(question.heading == "Stop digesting this sender?")
        #expect(question.confirm == "Stop digesting")
        #expect(question.destructive)
        let engine = ConfirmEngineSpy()
        question.answer(false, to: engine)
        #expect(engine.confirmed.isEmpty, "no is nothing")
        question.answer(true, to: engine)
        #expect(engine.confirmed == [9])
    }

    final class ConfirmEngineSpy: ConfirmEngine {
        var confirmed: [UInt64] = []
        func focusConfirmed(_ token: UInt64) { confirmed.append(token) }
    }
}
