import Testing

@testable import PostioKit

/// Asking for a window, twice (#1261).
@Suite struct WindowRequestTests {
    @Test func askingTwiceIsTwoRequests() {
        // The bug this type exists for: a `Bool` cannot say "again", so the
        // second `⌘,` after the window was closed looked like nothing had
        // changed and opened nothing.
        var request = WindowRequest(id: WindowId.settings)
        request.raise()
        let first = request
        request.raise()

        #expect(first != request, "a view watching this has to see a change")
        #expect(request.count == 2)
    }

    @Test func nobodyHasAskedUntilSomebodyAsks() {
        // A view watches the count; starting at zero is what stops it opening
        // a settings window on the first render.
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
