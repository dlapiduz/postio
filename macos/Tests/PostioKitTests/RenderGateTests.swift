import Testing

@testable import PostioKit

struct RenderGateTests {
    @Test func theLatestRenderIsTheCurrentOne() {
        let gate = RenderGate()
        let token = gate.begin()
        #expect(gate.isCurrent(token))
    }

    @Test func aSupersededRenderIsStale() {
        // The older result belongs to a message the user already left; drawing it puts one
        // message's text under another's header (#70).
        let gate = RenderGate()
        let first = gate.begin()
        let second = gate.begin()

        #expect(!gate.isCurrent(first))
        #expect(gate.isCurrent(second))
    }

    @Test func movingAwayAndBackStillSupersedes() {
        // A counter, not message ids: away and straight back gives the same id twice and the first
        // render is still stale.
        let gate = RenderGate()
        let toA = gate.begin()
        _ = gate.begin()  // to B
        let backToA = gate.begin()

        #expect(!gate.isCurrent(toA), "the first render of A is still stale")
        #expect(gate.isCurrent(backToA))
    }

    @Test func everyTokenIsDistinct() {
        let gate = RenderGate()
        let tokens = (0..<50).map { _ in gate.begin() }
        #expect(Set(tokens).count == tokens.count)
    }
}
