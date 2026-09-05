import PostioFFI
import Testing

@testable import PostioKit

/// What Postio says to a screen reader, and how much it moves.
///
/// The sentence is what can be wrong, so the sentence is what is asserted.
/// `accessibilityLabel` reads back whatever was last set whether or not
/// anything would ever speak it, so a test that set one and read it back would
/// be testing AppKit's property storage — the same trap
/// `docs/engineering-notes.md` records on the GTK side.
@Suite struct AccessibilityTests {
    private func presentation(
        sender: String = "ada@example.com",
        subject: String = "Quarterly figures",
        preview: String = "…the numbers we discussed on",
        unread: Bool = false,
        flagged: Bool = false,
        threadBadge: String? = nil,
        selected: Bool = false
    ) -> RowPresentation {
        RowPresentation(
            sender: sender,
            subject: subject,
            preview: preview,
            unread: unread,
            flagged: flagged,
            threadBadge: threadBadge,
            isPlaceholder: false,
            selected: selected
        )
    }

    @Test func aRowIsOneUtteranceAndNotFour() {
        let spoken = Announcements.row(presentation())
        #expect(spoken == "ada@example.com, Quarterly figures")
    }

    @Test func thePreviewIsNotRead() {
        // It is a fragment of the body, often mid-sentence, and reading it for
        // every row makes arrowing through a mailbox a wall of text. The
        // reading pane is what the body is for.
        let spoken = Announcements.row(presentation(preview: "…the numbers we discussed on"))
        #expect(!spoken.contains("numbers"))
    }

    @Test func theStatesThatChangeWhatYouWouldDoAreSpoken() {
        let spoken = Announcements.row(
            presentation(unread: true, flagged: true, threadBadge: "3", selected: true)
        )
        #expect(spoken.contains("unread"))
        #expect(spoken.contains("flagged"))
        #expect(spoken.contains("selected"))
        #expect(spoken.contains("3 messages"))
    }

    @Test func aRowThatHasNotArrivedSaysSoRatherThanNothing() {
        // An unlabelled row reads as "row", which sounds like a bug rather
        // than like a page still loading.
        #expect(Announcements.row(.placeholder) == "Loading")
    }

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

    @Test func reduceMotionRemovesTheTravelRatherThanShorteningIt() {
        // Asked for by people for whom movement is a symptom. A 50ms slide is
        // still a slide.
        #expect(Motion.duration(reduceMotion: true) == 0)
        #expect(Motion.duration(reduceMotion: false) <= 0.1, "PRODUCT.md §18's budget")
        #expect(Motion.duration(reduceMotion: false) > 0)
    }
}
