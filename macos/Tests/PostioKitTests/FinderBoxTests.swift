import Testing

@testable import PostioKit

/// The search box's modes: a prefix in the box chooses which question it
/// asks, as GTK's finder has it (`postio_ui::finder::MODES`). ⌘K opens the
/// box on `>`.
@Suite struct FinderBoxTests {
    @Test func eachPrefixAsksItsOwnQuestion() {
        #expect(FinderBox.asking(in: ">arch") == .init(mode: .command, text: "arch"))
        #expect(FinderBox.asking(in: "#inb") == .init(mode: .folder, text: "inb"))
        #expect(FinderBox.asking(in: "@ada") == .init(mode: .contact, text: "ada"))
        #expect(FinderBox.asking(in: "+tax") == .init(mode: .label, text: "tax"))
    }

    @Test func thePrefixAloneOffersEverything() {
        #expect(FinderBox.asking(in: ">") == .init(mode: .command, text: ""))
        #expect(FinderBox.asking(in: "#") == .init(mode: .folder, text: ""))
    }

    @Test func theSpaceAfterThePrefixIsNotPartOfTheName() {
        #expect(FinderBox.asking(in: "> arch") == .init(mode: .command, text: "arch"))
    }

    @Test func anythingElseIsASearch() {
        #expect(FinderBox.asking(in: "") == nil)
        #expect(FinderBox.asking(in: "from:ada") == nil)
        // Only a box that *starts* with a prefix has changed what it asks:
        // a search may contain the character.
        #expect(FinderBox.asking(in: "price >10") == nil)
        #expect(FinderBox.asking(in: "issue #12") == nil)
    }

    @Test func whatTheKeyPutsInTheBoxAsksForACommand() {
        #expect(FinderBox.asking(in: FinderBox.commands)?.mode == .command)
    }

    @Test func theHighlightStaysInTheList() {
        var box = FinderBox()
        box.move(by: -1, among: 5)
        #expect(box.highlighted == 0)
        box.move(by: 3, among: 5)
        box.move(by: 9, among: 5)
        #expect(box.highlighted == 4)
        box.move(by: 1, among: 0)
        #expect(box.highlighted == 0, "an empty list has nothing to highlight")
    }

    @Test func typingStartsFromTheTop() {
        var box = FinderBox()
        box.move(by: 2, among: 5)
        box.queryChanged()
        #expect(box.highlighted == 0)
    }
}
