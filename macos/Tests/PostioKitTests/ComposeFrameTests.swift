import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The composer's frame (specs/009-focus-macos T079; screens 05 and 06):
/// its title and when it was saved, the reply's quote folded, the word
/// count, Remind if no reply with its day, the From picker, and recipient
/// completion that shows nothing until a recipient is typed.
///
/// The words are the shared ones, through the boundary: nothing here is
/// composed in Swift.
@MainActor
@Suite struct ComposeFrameTests {
    static let quote = "On 2026-09-26, Lena Park wrote:\n> v3 is up.\n>\n> Comments welcome."

    static func draft(
        kind: DraftKindFfi = .reply, to: String = "lena@example.org", body: String = "\n\n" + quote
    ) -> DraftFfi {
        DraftFfi(
            id: 0, account: 1, kind: kind, from: "Mara Ostwald <mara@example.com>",
            to: to, cc: "", bcc: "", subject: "Re: Harbor API draft v3", body: body,
            bodyHtml: nil, rich: false, inReplyTo: 7, path: "/Users/someone/mail", attachments: [])
    }

    static let work = AccountFfi(
        id: 2, address: "mara@work.example", displayName: "Mara Ostwald", initials: "MO",
        isDefault: false, enabled: true, facts: [], needsAttention: false, repair: .nothing)

    @Test func theTitleNamesWhatIsBeingWritten() {
        #expect(ComposeModel(id: 1, draft: Self.draft(kind: .replyAll)).heading == "Reply to all")
        #expect(ComposeModel(id: 1, draft: Self.draft(kind: .new, to: "", body: "")).heading == "New message")
    }

    @Test func aReplysQuoteIsFoldedUnderWhatIsWrittenAndStillSent() throws {
        let model = ComposeModel(id: 1, draft: Self.draft())
        let fold = try #require(model.quoteFold, "the quote is folded")
        #expect(model.body == "\n\n", "the body field holds only what is written")
        #expect(fold.summary == "On 2026-09-26, Lena Park wrote \u{b7} 3 quoted lines")
        #expect(model.edited.body == "\n\n" + Self.quote, "what is sent still quotes")
        #expect(!model.isDirty, "folding is not an edit")

        model.body = "Thanks, v3 looks good.\n\n"
        #expect(model.edited.body == "Thanks, v3 looks good.\n\n" + Self.quote)

        model.showQuote()
        #expect(model.quoteFold == nil)
        #expect(model.body == "Thanks, v3 looks good.\n\n" + Self.quote, "unfolded, it is the body's")
        #expect(model.edited.body == model.body)
    }

    @Test func aNewMessageHasNothingToFold() {
        let model = ComposeModel(id: 1, draft: Self.draft(kind: .new, to: "", body: "Hi Ada,"))
        #expect(model.quoteFold == nil)
        #expect(model.body == "Hi Ada,")
    }

    @Test func theWordCountIsWhatWillBeSent() {
        let model = ComposeModel(id: 1, draft: Self.draft(kind: .new, to: "", body: ""))
        model.body = "Here are the Q4 numbers."
        #expect(model.summary == "Plain text \u{b7} 5 words")
    }

    @Test func remindIfNoReplyRidesOnTheDraftAndSaysItsDay() {
        let model = ComposeModel(id: 1, draft: Self.draft())
        #expect(model.remindAt == nil)
        #expect(model.remindWords == "Remind if no reply")
        let tuesday = Int64(Date().timeIntervalSince1970 * 1000) + 3 * 86_400_000
        model.remindAt = tuesday
        #expect(model.edited.remindAt == tuesday, "saved and sent with the draft")
        #expect(model.remindWords.hasPrefix("Remind if no reply \u{b7} "), "the day, when on")
        #expect(model.isDirty)
    }

    @Test func theTitleSaysWhenTheDraftWasSaved() {
        let model = ComposeModel(id: 1, draft: Self.draft())
        #expect(model.savedWords == nil, "nothing saved yet, nothing said")
        model.savedAt = Date()
        #expect(model.savedWords?.hasPrefix("Draft saved locally ") == true)
    }

    @Test func choosingAnotherAccountWritesFromIt() {
        let model = ComposeModel(id: 1, draft: Self.draft())
        model.choose(account: Self.work)
        #expect(model.edited.account == 2)
        #expect(model.edited.from == "Mara Ostwald <mara@work.example>")
        #expect(model.isDirty, "it is saved like any edit")
    }

    @Test func noContactListUntilARecipientIsTyped() {
        let model = ComposeModel(id: 1, draft: Self.draft())
        #expect(model.suggestions.isEmpty, "a reply opens with no list showing")
        #expect(model.suggesting == nil)
    }

    @Test func suggestionsAreForTheWordsStillInTheField() {
        let model = ComposeModel(id: 1, draft: Self.draft(kind: .new, to: "", body: ""))
        let grace = RecipientSuggestionFfi(
            label: "Grace Example <grace@example.org>", accepted: "Grace Example <grace@example.org>, ",
            group: false)
        let graham = RecipientSuggestionFfi(
            label: "Graham Example <graham@example.net>", accepted: "Graham Example <graham@example.net>, ",
            group: false)
        model.to = "gra"
        model.suggest([grace, graham], in: .to, for: "gr")
        #expect(model.suggestions.isEmpty, "an answer for words since changed is not shown")
        model.suggest([grace, graham], in: .to, for: "gra")
        #expect(model.suggesting == .to)
        #expect(model.highlighted == 0)

        model.moveSuggestion(by: 1)
        #expect(model.highlighted == 1)
        model.moveSuggestion(by: 1)
        #expect(model.highlighted == 1, "clamped at the last")

        #expect(model.acceptSuggestion())
        #expect(model.to == "Graham Example <graham@example.net>, ", "the field's whole text, as the engine said")
        #expect(model.suggestions.isEmpty)
        #expect(!model.acceptSuggestion(), "nothing left to accept")
    }

    @Test func dismissingTheListLeavesTheField() {
        let model = ComposeModel(id: 1, draft: Self.draft(kind: .new, to: "", body: ""))
        model.to = "gra"
        model.suggest(
            [RecipientSuggestionFfi(label: "Grace", accepted: "grace@example.org, ", group: false)],
            in: .to, for: "gra")
        model.dismissSuggestions()
        #expect(model.suggestions.isEmpty)
        #expect(model.to == "gra")
    }
}
