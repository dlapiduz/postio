import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// Quick Look's model (specs/010-focus-search T091; design §3.7, screen
/// 10). The controller decides what Quick Look shows and when it opens,
/// moves and closes (`crates/postio-focus/src/results.rs`); every
/// `FocusQuickLook` is the panel whole. What the Mac keeps is one panel:
/// a new view while it is up is new content in it, never a second panel.
@MainActor
struct QuickLookModelTests {
    final class Engine: QuickLookEngine {
        var invoked: [String] = []
        func invoke(_ id: String) { invoked.append(id) }
    }

    nonisolated static func view(_ position: String, current: UInt32? = 0) -> QuickLookViewFfi {
        QuickLookViewFfi(
            title: "Quick Look", position: position,
            walk: KeyHintFfi(key: "j/k", label: "moves through results while it stays open"),
            actions: [
                KeyHintFfi(key: "Return", label: "Open"), KeyHintFfi(key: "a", label: "Archive"),
                KeyHintFfi(key: "space", label: "Close"),
            ],
            subject: [RunFfi(text: "Atlas", highlighted: true, style: .plain)],
            sender: [RunFfi(text: "Ada Moreno", highlighted: false, style: .strong)],
            matchesLine: "2 matches in this conversation",
            matchesHint: KeyHintFfi(key: "]/[", label: "jump between them"),
            cards: [
                MatchCardFfi(place: "Body", when: "Ada · 26 Sep", passage: [], file: false),
                MatchCardFfi(place: "Subject", when: "", passage: [], file: false),
            ],
            current: current)
    }

    @Test func aNewViewWhileItIsUpIsNewContentInTheSamePanel() {
        let model = QuickLookModel(engine: Engine())
        #expect(model.apply(.focusQuickLook(view: Self.view("1 of 12"))) == .open)
        #expect(model.view?.position == "1 of 12")
        // j: the next result, in place.
        #expect(model.apply(.focusQuickLook(view: Self.view("2 of 12"))) == .redraw)
        #expect(model.view?.position == "2 of 12")
        // ]: the ring moved, nothing else.
        #expect(model.apply(.focusQuickLook(view: Self.view("2 of 12", current: 1))) == .redraw)
        #expect(model.view?.current == 1)
        #expect(model.apply(.focusQuickLook(view: nil)) == .close)
        #expect(model.view == nil)
        #expect(model.apply(.focusQuickLook(view: nil)) == nil, "closed once")
        #expect(model.apply(.focusLeaveResults) == nil, "not its event")
    }

    @Test func theHeadersButtonsAreTheKeysCommands() {
        let engine = Engine()
        let model = QuickLookModel(engine: engine)
        model.apply(.focusQuickLook(view: Self.view("1 of 12")))
        model.open()
        model.archive()
        model.close()
        #expect(engine.invoked == ["open_message", "archive", "quick_look"])
    }

    @Test func theHeaderSaysTheWalkAndTheMatchesInWords() {
        let view = Self.view("1 of 12")
        #expect(QuickLookWords.header(view) == "1 of 12 · j k moves through results while it stays open")
        #expect(QuickLookWords.matchesHint(view) == "· ] [ jump between them")
    }
}
