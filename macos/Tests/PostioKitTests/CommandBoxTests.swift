import Testing

@testable import PostioKit

/// The search box's command mode: `>` in the box, as GTK's finder has it
/// (`postio_ui::finder::MODES`), and what ⌘K opens.
@Suite struct CommandBoxTests {
    @Test func aBoxStartingWithTheMarkerIsAskingForACommand() {
        #expect(CommandBox.query(in: ">") == "")
        #expect(CommandBox.query(in: ">arch") == "arch")
        #expect(CommandBox.query(in: "> arch") == "arch", "the space after the marker is not part of the name")
    }

    @Test func anythingElseIsASearch() {
        #expect(CommandBox.query(in: "") == nil)
        #expect(CommandBox.query(in: "from:ada") == nil)
        // A search may contain the character; only a box that *starts*
        // with it has changed what it is asking.
        #expect(CommandBox.query(in: "price >10") == nil)
    }

    @Test func whatTheKeyPutsInTheBoxIsTheMarker() {
        #expect(CommandBox.query(in: CommandBox.opening) == "")
    }

    @Test func theHighlightStaysInTheList() {
        var box = CommandBox()
        box.move(by: -1, among: 5)
        #expect(box.highlighted == 0)
        box.move(by: 3, among: 5)
        box.move(by: 9, among: 5)
        #expect(box.highlighted == 4)
        box.move(by: 1, among: 0)
        #expect(box.highlighted == 0, "an empty list has nothing to highlight")
    }

    @Test func typingStartsFromTheTop() {
        var box = CommandBox()
        box.move(by: 2, among: 5)
        box.queryChanged()
        #expect(box.highlighted == 0)
    }
}
