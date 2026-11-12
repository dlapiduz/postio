import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The composer's thread half (spec 013 US7, T083): the forward chooser's
/// number keys, the older-reply bar's two actions, the forwarded messages
/// as rows, and the draft the controller asks for and then replaces.
@MainActor
@Suite struct ForwardChooserTests {
    static func draft(body: String = "", id: Int64 = 0) -> DraftFfi {
        DraftFfi(
            id: id, account: 1, kind: .forward, from: "Mara Ostwald <mara@example.com>",
            to: "", cc: "", bcc: "", subject: "Fwd: Harbor API draft v3", body: body, bodyHtml: nil,
            rich: false, inReplyTo: nil, forwardedFrom: 4, path: "/Users/someone/mail", attachments: [])
    }

    static let chooser = ChooserFfi(
        heading: "What to forward",
        cards: [
            ChooserCardFfi(form: .this, key: "1", title: "This message", detail: "Only this."),
            ChooserCardFfi(form: .thread, key: "2", title: "Whole thread", detail: "All 6."),
            ChooserCardFfi(form: .attachments, key: "3", title: "As attachments", detail: "Each .eml."),
        ],
        selected: .this)

    static let bar = OlderReplyFfi(
        title: "You're replying to message 4 of 6, not the latest", detail: "Two newer messages.",
        actions: [BarActionFfi(label: "Show them", key: "\u{2318}\u{21e7}\u{2193}"),
                  BarActionFfi(label: "Reply to latest", key: "\u{2325}E")],
        newerShown: false)

    final class Engine: ComposerEngine {
        var asked: [(DraftEditFfi, DraftFfi)] = []
        var newerAsked: [[Int64]] = []
        func focusComposerEdited() {}
        func focusDraftSaved(_ composition: UInt64, kept: Bool, error: String?) {}
        func focusSurfaceOpened(_ kind: SurfaceKindFfi) {}
        func focusSurfaceClosed(_ kind: SurfaceKindFfi) {}
        func focusComposerDraft(edit: DraftEditFfi, draft: DraftFfi) { asked.append((edit, draft)) }
        func focusNewerMessages(_ ids: [Int64]) -> [NewerMessageFfi] {
            newerAsked.append(ids)
            return ids.map { NewerMessageFfi(id: $0, who: "Lena Park", when: "Tue 22 Sep", text: "Six.") }
        }
    }

    // -- the chooser's keys ----------------------------------------------------

    @Test func eachCardIsItsNumberKeysCommandAndBack() {
        let forms: [ForwardFormFfi] = [.this, .thread, .attachments]
        let commands = forms.map(ForwardChooserKeys.command(for:))
        #expect(commands == ["forward_what_1", "forward_what_2", "forward_what_3"])
        #expect(commands.map(ForwardChooserKeys.form(for:)) == forms)
        #expect(ForwardChooserKeys.form(for: "send") == nil)
    }

    @Test func theChooserAnswersFirstOnlyWhileNothingIsBeingTypedIn() {
        #expect(
            ForwardChooserKeys.contexts(base: .composer, chooserShown: true, typing: false)
                == [.forwardChooser, .composer], "1 2 3 are the chooser's, the rest falls through")
        #expect(
            ForwardChooserKeys.contexts(base: .composer, chooserShown: true, typing: true)
                == [.composer], "a 2 typed into To is a 2")
        #expect(
            ForwardChooserKeys.contexts(base: .composer, chooserShown: false, typing: false) == [.composer])
        #expect(
            ForwardChooserKeys.contexts(base: .list, chooserShown: true, typing: false) == [.list],
            "never outside the composer")
    }

    @Test func theBarsTwoActionsAreShowThemAndReplyToLatest() {
        #expect(OlderReplyActions.command(at: 0) == "show_newer")
        #expect(OlderReplyActions.command(at: 1) == "reply_to_latest")
        #expect(OlderReplyActions.command(at: 2) == nil)
        #expect(Self.bar.actions.count == OlderReplyActions.commands.count)
    }

    // -- the forwarded messages as rows ------------------------------------------

    static let forwarded = """
        Ben, here is the thread.

        ---------- Forwarded messages ----------

        Ana Ruiz · 2026-09-21 09:12

        Kicking off v3 of the Harbor API.

        Lena Park · 2026-09-22 10:40

        Cursor pagination it is.
        """

    @Test func aForwardedThreadFoldsToOneRowPerMessageAndKeepsWhatWasTyped() throws {
        let fold = try #require(ForwardedRows.fold(Self.forwarded))
        #expect(fold.written == "Ben, here is the thread.\n\n")
        #expect(fold.summary == "2 forwarded messages")
        #expect(fold.written + fold.quote == Self.forwarded, "nothing is lost by folding")
        let rows = ForwardedRows.rows(in: fold.quote)
        #expect(rows.map(\.words) == [
            "1 · Ana Ruiz · 2026-09-21 09:12 · Kicking off v3 of the Harbor API.",
            "2 · Lena Park · 2026-09-22 10:40 · Cursor pagination it is.",
        ])
        #expect(ForwardedRows.fold("Just words.") == nil)
    }

    // -- the composer ------------------------------------------------------------

    @Test func theControllersViewsLandOnTheOpenComposer() {
        let window = ComposerWindow(engine: Engine())
        _ = window.apply(.focusComposer(kind: .forward, message: 4))
        window.show(Self.draft())
        #expect(window.apply(.focusForwardChooser(view: Self.chooser)) == .threadChanged)
        #expect(window.model?.chooser?.selected == .this)
        #expect(window.apply(.focusOlderReply(view: Self.bar)) == .threadChanged)
        #expect(window.model?.olderReply?.actions.count == 2)
        _ = window.apply(.focusOlderReply(view: nil))
        #expect(window.model?.olderReply == nil, "None takes the bar away")
    }

    @Test func showThemReadsTheNewerMessagesAndHidingPutsThemAway() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        _ = window.apply(.focusComposer(kind: .reply, message: 4))
        window.show(Self.draft())
        _ = window.apply(.focusNewerMessages(ids: [5, 6]))
        #expect(engine.newerAsked == [[5, 6]])
        #expect(window.model?.newer?.map(\.id) == [5, 6])
        _ = window.apply(.focusNewerMessages(ids: nil))
        #expect(window.model?.newer == nil)
    }

    @Test func theDraftTheControllerWantsIsWhatIsTypedNow() {
        let engine = Engine()
        let window = ComposerWindow(engine: engine)
        _ = window.apply(.focusComposer(kind: .forward, message: 4))
        window.show(Self.draft(body: "\n\n---------- Forwarded message ----------\nBen"))
        window.model?.body = "Typed words.\n\n---------- Forwarded message ----------\nBen"
        #expect(window.apply(.focusComposerDraftWanted(edit: .forwardThread)) == .threadChanged)
        #expect(engine.asked.first?.0 == .forwardThread)
        #expect(engine.asked.first?.1.body.hasPrefix("Typed words.") == true, "what was typed goes with it")
    }

    @Test func theDraftTheEngineChangedReplacesTheComposersAndKeepsItsOwn() throws {
        let window = ComposerWindow(engine: Engine())
        _ = window.apply(.focusComposer(kind: .forward, message: 4))
        window.show(Self.draft())
        var changed = Self.draft(body: Self.forwarded, id: 9)
        changed.subject = "Fwd: Harbor API draft v3 (6 messages)"
        changed.from = ""
        changed.path = ""
        #expect(window.apply(.focusComposerDraft(draft: changed)) == .threadChanged)
        let model = try #require(window.model)
        #expect(model.subject == "Fwd: Harbor API draft v3 (6 messages)")
        #expect(model.draft.id == 9)
        #expect(model.draft.from == "Mara Ostwald <mara@example.com>", "where it is written from stays")
        #expect(model.draft.path == "/Users/someone/mail")
        #expect(model.body == "Ben, here is the thread.\n\n", "the forwarded messages are folded to rows")
        #expect(model.forwardedRows.count == 2)
        #expect(model.plainText == Self.forwarded, "and are still in what is saved and sent")
        #expect(model.draft.forwardedFrom == 4)
    }

    @Test func quotedExpandedUnfoldsTheRowsIntoTheBodyAndFoldsThemAgain() throws {
        let window = ComposerWindow(engine: Engine())
        _ = window.apply(.focusComposer(kind: .forward, message: 4))
        window.show(Self.draft(body: Self.forwarded))
        let model = try #require(window.model)
        #expect(model.quoteFold != nil)
        _ = window.apply(.focusQuotedExpanded(expanded: true))
        #expect(model.quoteFold == nil)
        #expect(model.body == Self.forwarded)
        _ = window.apply(.focusQuotedExpanded(expanded: false))
        #expect(model.forwardedRows.count == 2)
        #expect(model.plainText == Self.forwarded)
    }

    @Test func aReplyToLatestKeepsTheWordsTypedAndMovesTheQuote() throws {
        let window = ComposerWindow(engine: Engine())
        _ = window.apply(.focusComposer(kind: .reply, message: 4))
        window.show(Self.draft(body: "Six is fine.\n\nOn Thu, Ben wrote:\n> old\n"))
        let model = try #require(window.model)
        #expect(model.body == "Six is fine.\n\n")
        var moved = Self.draft(body: "Six is fine.\n\nOn Fri, Lena wrote:\n> newer\n", id: 3)
        moved.inReplyTo = 6
        _ = window.apply(.focusComposerDraft(draft: moved))
        #expect(model.body == "Six is fine.\n\n", "the typed text is untouched, so is the caret")
        #expect(model.plainText.contains("> newer"))
        #expect(model.draft.inReplyTo == 6)
    }
}
