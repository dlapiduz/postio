import PostioFFI
import Testing

@testable import PostioKit

/// The message window's words, as its views draw them (specs/009-focus-macos
/// T067, T068): every word is the engine's (`focus_message_view`), every
/// keycap is spelled from the bindings in force, and the one decision the
/// Mac makes -- which verbs stand in the row and which wait in More -- is
/// the engine's `foldsIntoMore` applied to the engine's `folds`.
struct MessageWindowWordsTests {
    static let bindings: [String: String] = [
        "reply": "e", "reply_all": "E", "forward": "f", "archive": "a", "snooze": "s",
        "remind_if_no_reply": "h", "add_label": "l", "move": "m", "delete": "BackSpace",
        "more_actions": ".", "prev_message": "k", "next_message": "j",
        "prev_in_conversation": "[", "next_in_conversation": "]", "dismiss_marker": "-",
        "switch_treatment": "O",
    ]

    static func binding(_ command: String) -> String? { bindings[command] }

    static func verb(_ command: String, _ label: String, folds: Bool = false) -> FocusVerbFfi {
        FocusVerbFfi(command: command, label: label, folds: folds)
    }

    static let row: [FocusVerbFfi] = [
        verb("reply", "Reply"), verb("reply_all", "Reply all"), verb("forward", "Forward"),
        verb("archive", "Archive"), verb("snooze", "Snooze"), verb("remind_if_no_reply", "Remind"),
        verb("add_label", "Label", folds: true), verb("move", "Move", folds: true),
        verb("delete", "Delete", folds: true),
    ]

    static func view(
        actions: [FocusVerbFfi] = row,
        more: FocusRowActionFfi? = FocusRowActionFfi(command: "more_actions", label: "More"),
        marker: MarkerLineFfi? = nil
    ) -> FocusMessageViewFfi {
        FocusMessageViewFfi(
            message: 7, subject: "Harbor API draft v3",
            position: "Message 5 of 60 \u{b7} thread of 6",
            thread: FocusThreadChipFfi(
                text: "Latest of 6 in this thread",
                earlier: FocusRowActionFfi(command: "prev_in_conversation", label: "earlier message"),
                later: nil),
            earlier: 6, later: nil,
            labels: [LabelPillFfi(name: "Harbor", color: "#3a9a6b")],
            addLabel: FocusRowActionFfi(command: "add_label", label: "+ Label"),
            fields: [
                FocusFieldFfi(
                    field: "From", people: [FocusPersonFfi(name: "Lena Park", address: "lena@example.com")],
                    more: nil, all: "Lena Park <lena@example.com>"),
            ],
            date: "Today, 15:22", marker: marker,
            dismiss: FocusRowActionFfi(command: "dismiss_marker", label: "Dismiss"),
            actions: actions, more: more,
            attachments: [FocusAttachmentFfi(id: 3, name: "draft-v3.pdf", size: "47 KB")])
    }

    @Test func theTitleAreaIsTheSubjectThePositionAndOneKJCap() {
        let words = MessageChromeWords(view: Self.view(), folds: false, binding: Self.binding)
        #expect(words.title == "Harbor API draft v3")
        #expect(words.position == "Message 5 of 60 \u{b7} thread of 6")
        #expect(words.stepCap == "k j", "one cap for the pair, previous first, as screen 04")
        #expect(words.previous == "prev_message" && words.next == "next_message")
    }

    @Test func aWideWindowDrawsEveryVerbAndNoMore() {
        let words = MessageChromeWords(view: Self.view(), folds: false, binding: Self.binding)
        #expect(words.row.map(\.label) == [
            "Reply", "Reply all", "Forward", "Archive", "Snooze", "Remind", "Label", "Move", "Delete",
        ])
        #expect(words.row.map(\.cap) == ["e", "⇧E", "f", "a", "s", "h", "l", "m", "⌫"])
        #expect(words.more == nil)
        #expect(words.folded.isEmpty)
    }

    @Test func aNarrowWindowFoldsLabelMoveAndDeleteIntoMore() {
        let words = MessageChromeWords(view: Self.view(), folds: true, binding: Self.binding)
        #expect(words.row.map(\.command) == [
            "reply", "reply_all", "forward", "archive", "snooze", "remind_if_no_reply",
        ])
        #expect(words.more?.label == "More" && words.more?.cap == ".")
        #expect(words.folded.map(\.command) == ["add_label", "move", "delete"])
    }

    @Test func aDraftOnItsWayDrawsOnlyTheVerbsThatSettleIt() {
        // `send_verbs`: nothing folds and there is no More.
        let view = Self.view(
            actions: [Self.verb("retry_send", "Retry send"), Self.verb("open_message", "Edit")],
            more: nil)
        let words = MessageChromeWords(view: view, folds: true, binding: Self.binding)
        #expect(words.row.map(\.label) == ["Retry send", "Edit"])
        #expect(words.more == nil)
    }

    @Test func theHeaderBlockCarriesTheEnginesWordsAndTheKeys() {
        let marker = MarkerLineFfi(
            chip: "To-do", date: "Wed 30 Sep", quote: "Please leave comments by Wednesday",
            status: nil, actions: [FocusRowActionFfi(command: "snooze", label: "Snooze")])
        let words = MessageHeaderWords(view: Self.view(marker: marker), binding: Self.binding)
        #expect(words.thread?.text == "Latest of 6 in this thread")
        #expect(words.thread?.earlier?.cap == "[")
        #expect(words.thread?.later == nil)
        #expect(words.subject == "Harbor API draft v3")
        #expect(words.labels.map(\.name) == ["Harbor"])
        #expect(words.labels.first?.colour != nil, "the label's own dot")
        #expect(words.addLabel.label == "+ Label" && words.addLabel.cap == "l")
        #expect(words.fields.first?.people.first?.address == "lena@example.com")
        #expect(words.date == "Today, 15:22")
        let card = try? #require(words.card)
        #expect(card?.chip == "To-do")
        #expect(card?.quoted == "\u{201c}Please leave comments by Wednesday\u{201d}")
        #expect(card?.sentence == "Please leave comments by Wednesday", "what the body highlights")
        #expect(card?.actions.map(\.cap) == ["s"])
        #expect(card?.dismiss.label == "Dismiss" && card?.dismiss.cap == "-")
        #expect(words.attachments.map(\.name) == ["draft-v3.pdf"])
    }

    @Test func noMarkerNoCard() {
        let words = MessageHeaderWords(view: Self.view(), binding: Self.binding)
        #expect(words.card == nil)
    }

    @Test func theRenderModeLineIsTheEnginesWithItsKey() {
        let mode = RenderModeWordsFfi(
            title: "App colours", detail: "sender colours and fonts removed",
            action: "Show original", offerAlways: true, always: "Always for this sender")
        let words = RenderModeLine(mode, binding: Self.binding)
        #expect(words.title == "App colours")
        #expect(words.action.label == "Show original")
        #expect(words.action.command == "switch_treatment")
        #expect(words.action.cap == "⇧O")
        #expect(words.always == "Always for this sender")
    }
}
