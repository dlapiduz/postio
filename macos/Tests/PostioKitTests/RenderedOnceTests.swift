import Testing
@testable import PostioKit

@Suite struct RenderedOnceTests {
    @Test func nothingRendersUntilSomebodyAsks() {
        // The product's default, not a convenience: fetching one picture
        // tells the sender the message was opened, when, and roughly where
        // from.
        let rendered = RenderedOnce()
        #expect(!rendered.isOn(7))
    }

    @Test func renderingOneMessageSaysNothingAboutTheNext() {
        // Per message. A standing "show me everything" is the popover's
        // grant, and it is a different gesture with a different surface.
        var rendered = RenderedOnce()
        rendered.render(7)
        #expect(rendered.isOn(7))
        #expect(!rendered.isOn(8))
    }

    @Test func renderingCannotBeTakenBack() {
        // The pictures have been fetched. Hiding them again would claim
        // something Postio cannot make true.
        var rendered = RenderedOnce()
        rendered.render(7)
        rendered.render(7)
        #expect(rendered.isOn(7))
    }

    @Test func leavingTheConversationForgetsIt() {
        // "Once" means this view. The next message opens blocked again —
        // which is what makes the key safe to press.
        var rendered = RenderedOnce()
        rendered.render(7)
        rendered.clear()
        #expect(!rendered.isOn(7))
    }
}
