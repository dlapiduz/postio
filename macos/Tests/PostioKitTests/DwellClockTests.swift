import Foundation
import Testing

@testable import PostioKit

/// Resting marks; sweeping does not (#71, #1159).
///
/// Time is injected rather than waited for, so these assert the *rule* and
/// run in microseconds. The rule is the one that matters: unread state is
/// only a signal while it means "you have not looked at this", and a client
/// that marks on arrival destroys it the first time somebody scrolls a
/// mailbox end to end.
@MainActor
@Suite struct DwellClockTests {
    /// A scheduler a test drives by hand.
    ///
    /// `@MainActor` like the clock it feeds: the whole thing lives on one
    /// actor, so there is no concurrency here to be unsafe about — the only
    /// reason time is injected at all is to assert the rule without waiting
    /// a real second per case.
    @MainActor
    final class Fake {
        private var pending: [(id: Int, work: () -> Void)] = []
        private var next = 0
        private(set) var cancelled = 0

        func schedule(_: TimeInterval, _ work: @escaping () -> Void) -> DwellClock.Cancellable {
            next += 1
            let id = next
            pending.append((id, work))
            return Handle(fake: self, id: id)
        }

        /// Let every armed clock that was not cancelled fire.
        func advance() {
            let due = pending
            pending = []
            for entry in due { entry.work() }
        }

        func drop(_ id: Int) {
            let before = pending.count
            pending.removeAll { $0.id == id }
            cancelled += before - pending.count
        }

        /// `@MainActor` for the same reason the fake is: the clock cancels
        /// from the actor it lives on, and the protocol's `cancel` is not
        /// isolated because the production handle wraps a `DispatchWorkItem`
        /// that is safe anywhere.
        struct Handle: DwellClock.Cancellable {
            let fake: Fake
            let id: Int
            func cancel() {
                MainActor.assumeIsolated { fake.drop(id) }
            }
        }
    }

    private func clock(_ fake: Fake, marked: @escaping (Int64) -> Void) -> DwellClock {
        DwellClock(delay: 1, schedule: { fake.schedule($0, $1) }, fired: marked)
    }

    @Test func restingOnAMessageMarksIt() {
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        dwell.cursorMoved(to: 7)
        fake.advance()

        #expect(marked == [7])
    }

    @Test func sweepingThroughAMailboxMarksNothing() {
        // The rule. Holding `j` re-arms on every row and rests on none, so a
        // sweep of fifty messages must mark zero of them — anything else
        // destroys unread state as a signal, which is the one thing it is for.
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        for row in Int64(1)...50 {
            dwell.cursorMoved(to: row)
        }
        // Only the last one is still armed; the other forty-nine were
        // cancelled as the cursor left them.
        fake.advance()

        #expect(marked == [50], "a sweep marked \(marked.count) messages")
    }

    @Test func aCursorOnNothingArmsNothing() {
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        dwell.cursorMoved(to: nil)
        fake.advance()

        #expect(marked.isEmpty)
    }

    @Test func stoppingCancelsAClockInFlight() {
        // The window losing focus, or an overlay taking over: the message is
        // no longer in front of anybody, so the clock must not fire.
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        dwell.cursorMoved(to: 7)
        dwell.stop()
        fake.advance()

        #expect(marked.isEmpty)
    }

    @Test func theMessageMarkedIsTheOneTheClockStartedOn() {
        // The cursor may move between the timer firing and the mark running.
        // What was read is what the clock was armed for, not wherever the
        // cursor happens to be now.
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        dwell.cursorMoved(to: 7)
        fake.advance()
        dwell.cursorMoved(to: 9)

        #expect(marked == [7])
    }
}
