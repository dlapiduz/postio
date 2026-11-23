import Testing
@testable import PostioKit

@Suite struct RenderedOnceTests {
    @Test func nothingRendersUntilSomebodyAsks() {
        let rendered = RenderedOnce()
        #expect(!rendered.isOn(7))
    }

    @Test func renderingOneMessageSaysNothingAboutTheNext() {
        var rendered = RenderedOnce()
        rendered.render(7)
        #expect(rendered.isOn(7))
        #expect(!rendered.isOn(8))
    }

    @Test func renderingCannotBeTakenBack() {
        // The pictures have been fetched; hiding them again would claim something Postio cannot
        // make true.
        var rendered = RenderedOnce()
        rendered.render(7)
        rendered.render(7)
        #expect(rendered.isOn(7))
    }

    @Test func leavingTheConversationForgetsIt() {
        var rendered = RenderedOnce()
        rendered.render(7)
        rendered.clear()
        #expect(!rendered.isOn(7))
    }
}
