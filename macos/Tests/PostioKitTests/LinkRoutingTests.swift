import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// `postio://message/<id>`, the link a captured task carries back to its
/// message (specs/009-focus-macos T117, C21).
///
/// The reading is the boundary's (`parse_message_link`, over
/// `postio_ui::links`), so a link reads here exactly as the line capture
/// wrote it. What this decides is where each one goes: a message link to
/// the controller, which opens it or says it is gone; anything else of
/// Postio's straight to the pill, in the boundary's words.
@MainActor
@Suite struct LinkRoutingTests {
    @Test func aMessageLinkGoesToTheControllerAsWritten() throws {
        let url = try #require(URL(string: "postio://message/42/"))
        #expect(PostioLink.route(url) == .open(uri: "postio://message/42/", message: 42))
    }

    @Test func theLinkCaptureWritesComesBackToItsMessage() throws {
        // The round trip: what a captured line carries is what opens.
        let url = try #require(URL(string: messageLink(message: 1_234)))
        #expect(PostioLink.route(url) == .open(uri: messageLink(message: 1_234), message: 1_234))
    }

    @Test func aLinkToNoMessageIsSaidInThePill() throws {
        for link in ["postio://message/0", "postio://message/", "postio://settings", "POSTIO://elsewhere/7"] {
            let url = try #require(URL(string: link))
            #expect(PostioLink.route(url) == .unknown(linkUnknown()), "\(link)")
        }
    }

    @Test func aLinkThatIsNotPostiosIsNotRouted() throws {
        #expect(PostioLink.route(try #require(URL(string: "mailto:ada@example.com"))) == nil)
        #expect(PostioLink.route(try #require(URL(string: "https://example.com/message/4"))) == nil)
    }

    @Test func theUnknownSentenceIsThePillsAsTheControllerWouldSayIt() {
        // Said by the Mac without a round trip, so it must be what the
        // controller's own refusal is: the same words, a notice, no Undo.
        let event = PostioLink.toast(linkUnknown())
        let focus = FocusIntents()
        #expect(focus.apply(event) == .toast)
        #expect(focus.toast?.text == linkUnknown())
        #expect(focus.toast?.kind == .notice)
        #expect(focus.toast?.offersUndo == false)
    }

    @Test func linksThatArriveBeforeTheStoreOpensWaitForIt() throws {
        // Clicking a captured line launches Postio cold: the link arrives
        // before the session does, and opening it then would open nothing.
        var waiting = PostioLink.Waiting()
        let first = try #require(URL(string: "postio://message/7"))
        let second = try #require(URL(string: "postio://message/8"))
        waiting.hold(.open(uri: first.absoluteString, message: 7))
        waiting.hold(.open(uri: second.absoluteString, message: 8))

        #expect(waiting.take() == [
            .open(uri: "postio://message/7", message: 7),
            .open(uri: "postio://message/8", message: 8),
        ])
        #expect(waiting.take().isEmpty, "a link is opened once")
    }
}
