import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// Quick Look's panel (specs/010-focus-search T091; design §3.7, screen
/// 10): a floating panel over the results, 780 by 470, radius 14, with a
/// shadow and nothing dimmed. One panel: a new view keeps it and changes
/// only what it holds. Nothing is put on screen -- `present` is replaced.
@MainActor
struct QuickLookPanelTests {
    final class Engine: QuickLookEngine {
        func invoke(_ id: String) {}
    }

    static func view(_ position: String) -> QuickLookViewFfi {
        QuickLookViewFfi(
            title: "Quick Look", position: position, walk: nil, actions: [], subject: [], sender: [],
            matchesLine: "1 match in this conversation", matchesHint: nil,
            cards: [MatchCardFfi(place: "Body", when: "Ada · 26 Sep", passage: [], file: false)], current: 0)
    }

    static func main() -> NSWindow {
        let frame = NSRect(x: 100, y: 100, width: 1440, height: 900)
        let window = NSWindow(
            contentRect: frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.setFrame(frame, display: false)
        return window
    }

    @Test func itOpensOncePlacedOverTheResultsAndANewViewKeepsThePanel() throws {
        let model = QuickLookModel(engine: Engine())
        var presented = 0
        let panel = QuickLookPanel(model: model, present: { _, _ in presented += 1 })
        let main = Self.main()

        panel.apply(try #require(model.apply(.focusQuickLook(view: Self.view("1 of 12")))), over: main)
        let first = try #require(panel.panel)
        #expect(presented == 1)
        #expect(first.frame.size == NSSize(width: 780, height: 470))
        // Centred across the window, 160 under its top (screen 10: 330, 160
        // in a 1440 by 900 window).
        #expect(first.frame.origin == NSPoint(x: 100 + 330, y: 100 + 900 - 160 - 470))
        #expect(first.hasShadow)
        #expect(!first.isOpaque, "its corners are the window's own: radius 14")
        #expect(!first.canBecomeKey, "the results keep the keyboard")

        panel.apply(try #require(model.apply(.focusQuickLook(view: Self.view("2 of 12")))), over: main)
        #expect(panel.panel === first, "the same panel")
        #expect(presented == 1, "not presented again")

        panel.apply(try #require(model.apply(.focusQuickLook(view: nil))), over: main)
        #expect(panel.panel == nil)
    }
}
