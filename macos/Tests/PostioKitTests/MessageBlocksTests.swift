import PostioFFI
import Testing

@testable import PostioKit

/// What the message window's content column draws, block by block, for a
/// message that has a banner, a caveat, both or neither (#1585).
///
/// The column draws exactly `MessageBlock.blocks` and nothing beside it, so
/// this is the list of what is on screen rather than of what the view was
/// handed. The banner is the case that needed it: the boundary carried the
/// offer and its tests passed while Focus's message window had no banner at
/// all.
@Suite struct MessageBlocksTests {
    static let summary = "This message is from news.example.org"
    static let caveat = "Parts of this message could not be decoded."
    static let offer = UnsubscribeOfferFfi(
        listIdentifier: "news.example.org", summary: summary, action: "Unsubscribe")

    static func words() -> MessageHeaderWords {
        let action = FocusRowActionFfi(command: "dismiss_marker", label: "Dismiss")
        let view = FocusMessageViewFfi(
            message: 7, subject: "Harbor API draft v3", position: "Message 1 of 3",
            thread: nil, earlier: nil, later: nil, labels: [], addLabel: action,
            fields: [
                FocusFieldFfi(
                    field: "From",
                    people: [FocusPersonFfi(name: "Ada Lovelace", address: "ada@example.com")],
                    more: nil, all: "Ada Lovelace <ada@example.com>")
            ],
            date: "Today, 15:22", marker: nil, dismiss: action, actions: [],
            more: nil, attachments: [])
        return MessageHeaderWords(view: view, binding: { _ in nil })
    }

    static func document(
        unsubscribe: UnsubscribeOfferFfi? = nil, caveat: String? = nil,
        senderWarning: String? = nil
    ) -> FocusReaderDocumentFfi {
        FocusReaderDocumentFfi(
            html: "<p>7</p>", notice: nil, caveat: caveat, senderWarning: senderWarning, unsubscribe: unsubscribe,
            treatmentShown: .appColours, treatmentClassified: .appColours, renderMode: nil,
            sender: "ada@example.com", senderChoice: nil, windowWidth: 720, columnWidth: 560,
            foldsIntoMore: false, paperFloor: 0.85)
    }

    static func blocks(
        _ document: FocusReaderDocumentFfi?, state: Unsubscribing.State = .offered
    ) -> [MessageBlock] {
        MessageBlock.blocks(words: words(), document: document, unsubscribe: state)
    }

    @Test func theHeaderSaysWhatItIsAboutAndWhoItIsFrom() {
        let drawn = Self.blocks(Self.document())
        #expect(drawn.contains(.subject("Harbor API draft v3")))
        #expect(drawn.contains(.sender))
        #expect(Self.words().fields.first?.people.first?.name == "Ada Lovelace")
        #expect(Self.words().date == "Today, 15:22")
        #expect(drawn.firstIndex(of: .sender)! < drawn.firstIndex(of: .body)!)
    }

    @Test func listMailOffersUnsubscribeBetweenTheHeaderAndTheBody() {
        let drawn = Self.blocks(Self.document(unsubscribe: Self.offer))
        let banner = drawn.firstIndex(of: .unsubscribe(Self.offer, .offered))
        #expect(banner != nil, "\(drawn)")
        if let banner {
            #expect(banner > drawn.firstIndex(of: .sender)!)
            #expect(banner < drawn.firstIndex(of: .body)!)
        }
    }

    @Test func personalMailHasNoBanner() {
        // The control for the test above, over the same header.
        let drawn = Self.blocks(Self.document())
        #expect(drawn.contains(.body), "the column was composed")
        #expect(!drawn.contains { if case .unsubscribe = $0 { true } else { false } })
    }

    @Test func theBannerSaysWhatItIsDoingAndGoesOnceTheListIsLeft() {
        #expect(
            Self.blocks(Self.document(unsubscribe: Self.offer), state: .leaving)
                .contains(.unsubscribe(Self.offer, .leaving)))
        let left = Self.blocks(Self.document(unsubscribe: Self.offer), state: .left)
        #expect(left.contains(.body))
        #expect(
            !left.contains { if case .unsubscribe = $0 { true } else { false } },
            "its absence is the confirmation")
    }

    @Test func aBodyThatLostAPartSaysSoUnderIt() {
        let lost = Self.blocks(Self.document(caveat: Self.caveat))
        let caveat = lost.firstIndex(of: .caveat(Self.caveat))
        #expect(caveat != nil, "\(lost)")
        if let caveat { #expect(caveat > lost.firstIndex(of: .body)!) }
        #expect(!Self.blocks(Self.document()).contains { if case .caveat = $0 { true } else { false } })
    }

    @Test func aDoubtedSenderGetsOneSentenceUnderTheirHeaderAndOrdinaryMailGetsNone() {
        let said = "This message failed sender verification: it may not be from example.org."
        let drawn = Self.blocks(Self.document(senderWarning: said))
        let at = drawn.firstIndex(of: .senderWarning(said))
        #expect(at != nil, "\(drawn)")
        if let at {
            #expect(at > drawn.firstIndex(of: .sender)!)
            #expect(at < drawn.firstIndex(of: .body)!)
        }
        #expect(!Self.blocks(Self.document()).contains { if case .senderWarning = $0 { true } else { false } })
    }

    @Test func aMessageNotYetReadDrawsItsHeaderAndNoBody() {
        let drawn = Self.blocks(nil)
        #expect(drawn.contains(.subject("Harbor API draft v3")))
        #expect(!drawn.contains(.body))
    }

    @Test func theBannerSitsOnTheRhythmGTKsNoticeSlotDoes() {
        // 12 under the sender block, and the 24 to the body is then the
        // notice's (`postio_ui::focus_dialog::rhythm::CARD_TO_NOTICE`).
        let banner = MessageBlock.unsubscribe(Self.offer, .offered)
        #expect(MessageBlock.gap(above: banner, after: .sender) == 12)
        #expect(MessageBlock.gap(above: .body, after: banner) == MessageRhythm.cardToBody)
    }
}
