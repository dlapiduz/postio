import Foundation
import Testing

@testable import PostioKit

@MainActor
@Suite struct DwellClockTests {
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

        struct Handle: DwellClock.Cancellable {
            let fake: Fake
            let id: Int
            func cancel() {
                MainActor.assumeIsolated { fake.drop(id) }
            }
        }
    }

    private func clock(_ fake: Fake, marked: @escaping (Int64) -> Void) -> DwellClock {
        DwellClock(schedule: { fake.schedule($0, $1) }, fired: marked)
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
        // Holding `j` re-arms on every row and rests on none, so a sweep of fifty messages must
        // mark zero: unread state is a signal.
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        for row in Int64(1)...50 {
            dwell.cursorMoved(to: row)
        }
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
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        dwell.cursorMoved(to: 7)
        dwell.stop()
        fake.advance()

        #expect(marked.isEmpty)
    }

    @Test func theMessageMarkedIsTheOneTheClockStartedOn() {
        // What was read is what the clock was armed for, not wherever the cursor is now.
        let fake = Fake()
        var marked: [Int64] = []
        let dwell = clock(fake) { marked.append($0) }

        dwell.cursorMoved(to: 7)
        fake.advance()
        dwell.cursorMoved(to: 9)

        #expect(marked == [7])
    }
}
