import Testing

@testable import PostioKit

/// *View original* — the one gesture that leaves reader view.
///
/// `⌘O` was in the menu with its chord, in the palette, and reached nothing.
/// The capability was already whole on both sides: the boundary takes an
/// `original` flag and renders what the sender wrote on their own sheet, and
/// the conversation pane's `⋯` menu already had a toggle for it. What was
/// missing was a *command* route — so the key did nothing while the menu item
/// beside it worked, which reads as an application that half-works rather
/// than one with a feature missing.
///
/// **Per message and per view**, which is the rule worth keeping honest:
/// nothing is remembered, so the next message opens reduced again. Bulk mail
/// is what reader view is *for*, and a standing "show me originals" would
/// quietly turn it off for everything.
@Suite struct OriginalViewTests {
    @Test func aMessageStartsReduced() {
        var showing = OriginalView()
        #expect(!showing.isOn(7))
    }

    @Test func togglingOneMessageLeavesTheRestReduced() {
        var showing = OriginalView()
        showing.toggle(7)
        #expect(showing.isOn(7))
        #expect(!showing.isOn(8), "a conversation is several messages and this is about one")
    }

    @Test func togglingTwiceComesBack() {
        var showing = OriginalView()
        showing.toggle(7)
        showing.toggle(7)
        #expect(!showing.isOn(7))
    }

    @Test func leavingTheConversationForgetsEverything() {
        // "Per view": the next conversation opens reduced whatever was done
        // to the last one. A grant that outlived the pane would be a standing
        // preference nobody set.
        var showing = OriginalView()
        showing.toggle(7)
        showing.toggle(8)
        showing.clear()
        #expect(!showing.isOn(7))
        #expect(!showing.isOn(8))
    }
}
