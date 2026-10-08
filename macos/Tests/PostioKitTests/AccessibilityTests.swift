import PostioFFI
import Testing

@testable import PostioKit

/// What Postio says to a screen reader, and how much it moves.
///
/// The sentence is what can be wrong, so the sentence is what is asserted.
/// `accessibilityLabel` reads back whatever was last set whether or not
/// anything would ever speak it, so a test that set one and read it back would
/// be testing AppKit's property storage — the same trap
/// `docs/archive/engineering-notes.md` records on the GTK side.
@Suite struct AccessibilityTests {
    @Test func theFocusOrderIsTheVisualOrder() {
        // A focus order that disagrees with the layout is the classic way a
        // keyboard-first application becomes unusable without a mouse.
        #expect(Pane.allCases == [.sidebar, .list, .reader])
        #expect(Pane.sidebar.next() == .list)
        #expect(Pane.list.next() == .reader)
        #expect(Pane.reader.next() == .sidebar, "the cycle has to come back round")
        #expect(Pane.sidebar.next(false) == .reader)
    }

    @Test func everyPaneResolvesKeysAsItself() {
        // The keyboard's context follows focus, or `j` in the sidebar moves
        // the message list.
        #expect(Pane.sidebar.context == .sidebar)
        #expect(Pane.list.context == .list)
        #expect(Pane.reader.context == .reader)
    }

    @Test func everyPaneIsNamed() {
        // A pane a screen reader calls "group" is a pane nobody can navigate
        // to on purpose.
        for pane in Pane.allCases {
            #expect(!pane.label.isEmpty)
        }
    }

    @Test func everyInterceptedCommandIsInTheRegistry() {
        // The frontend presents a surface for these rather than sending them
        // on, and it matches them by *string*. A literal that no longer names
        // a registered command is a key that silently does nothing — `/` not
        // opening search, and no error anywhere to say why. The registry is
        // the authority, so ask it.
        let known = Set(PostioRegistry.commands.map(\.id))
        for id in Intercepted.all {
            #expect(known.contains(id), "`\(id)` is intercepted and is not a command")
        }
    }
}
