import Testing

@testable import PostioKit

@Suite struct WindowRequestTests {
    @Test func askingTwiceIsTwoRequests() {
        var request = WindowRequest(id: WindowId.settings)
        request.raise()
        let first = request
        request.raise()

        #expect(first != request, "a view watching this has to see a change")
        #expect(request.count == 2)
    }

    @Test func nobodyHasAskedUntilSomebodyAsks() {
        // A view watches the count; starting at zero stops it opening a settings window on first
        // render.
        let request = WindowRequest(id: WindowId.settings)
        #expect(!request.wasRaised)
        #expect(request.count == 0)
    }

    @Test func theIdIsTheOneOpenWindowTakes() {
        var request = WindowRequest(id: WindowId.settings)
        request.raise()
        #expect(request.id == "settings")
    }
}
