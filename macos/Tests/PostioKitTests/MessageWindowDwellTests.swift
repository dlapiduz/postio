import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// A message left open in the message window is read; one stepped past, or
/// closed before the clock ran out, is not (#71, #1159, and again after
/// spec 009 rebuilt the Mac as Focus and the wiring went with the old list).
///
/// The rule and the delay are `postio_ui::dwell`'s, through `DwellClock`.
/// What these pin is the window's half, mirroring GTK's `open.rs`: the
/// clock follows the message on screen, stops when the window closes or
/// Postio is not the active app, starts again on its return, and leaves a
/// message alone once the person has set its read state.
@MainActor
struct MessageWindowDwellTests {
    /// A scheduler a test drives by hand.
    @MainActor
    final class Clock {
        private var pending: [(id: Int, work: () -> Void)] = []
        private var next = 0

        func schedule(_: TimeInterval, _ work: @escaping () -> Void) -> DwellClock.Cancellable {
            next += 1
            pending.append((next, work))
            return Handle(clock: self, id: next)
        }

        /// Let every clock still armed run out.
        func advance() {
            let due = pending
            pending = []
            for entry in due { entry.work() }
        }

        func drop(_ id: Int) { pending.removeAll { $0.id == id } }

        struct Handle: DwellClock.Cancellable {
            let clock: Clock
            let id: Int
            func cancel() { MainActor.assumeIsolated { clock.drop(id) } }
        }
    }

    final class Marked { var ids: [Int64] = [] }

    static func model() -> (MessageWindowModel, Clock, Marked) {
        let clock = Clock()
        let marked = Marked()
        let model = MessageWindowModel(
            source: MessageWindowModelTests.Source(),
            report: { _, _ in },
            markRead: { marked.ids.append($0) },
            schedule: { clock.schedule($0, $1) })
        return (model, clock, marked)
    }

    @Test func aMessageLeftOpenIsMarkedRead() {
        let (model, clock, marked) = Self.model()
        model.open(message: 7, index: 0, total: 3, mainWidth: 1440)
        #expect(marked.ids.isEmpty, "not on opening: only once it has been read")
        clock.advance()
        #expect(marked.ids == [7])
    }

    @Test func steppingPastAMessageLeavesItUnread() {
        let (model, clock, marked) = Self.model()
        model.open(message: 7, index: 0, total: 3, mainWidth: 1440)
        model.open(message: 8, index: 1, total: 3, mainWidth: 1440)
        model.open(message: 9, index: 2, total: 3, mainWidth: 1440)
        clock.advance()
        #expect(marked.ids == [9], "only the message the window rested on")
    }

    @Test func closingBeforeTheClockRunsOutLeavesItUnread() {
        let (model, clock, marked) = Self.model()
        model.open(message: 7, index: 0, total: 3, mainWidth: 1440)
        model.closed()
        clock.advance()
        #expect(marked.ids.isEmpty)
    }

    @Test func aMessageIsNotReadWhilePostioIsInTheBackground() {
        let (model, clock, marked) = Self.model()
        model.open(message: 7, index: 0, total: 3, mainWidth: 1440)
        model.setAppActive(false)
        clock.advance()
        #expect(marked.ids.isEmpty, "the clock stops with the app in the background")
        model.open(message: 8, index: 1, total: 3, mainWidth: 1440)
        clock.advance()
        #expect(marked.ids.isEmpty, "and does not start there either")
        model.setAppActive(true)
        clock.advance()
        #expect(marked.ids == [8], "back in front, it starts again on the message shown")
    }

    @Test func aMessageAlreadyReadIsNotMarkedAgainOnReturn() {
        let (model, clock, marked) = Self.model()
        model.open(message: 7, index: 0, total: 3, mainWidth: 1440)
        clock.advance()
        model.setAppActive(false)
        model.setAppActive(true)
        clock.advance()
        #expect(marked.ids == [7])
    }

    @Test func readStateSetByThePersonIsNotOverruled() {
        // `u` on an open message keeps it unread while it stays open (T237).
        let (model, clock, marked) = Self.model()
        model.open(message: 7, index: 0, total: 3, mainWidth: 1440)
        model.readStateSet()
        clock.advance()
        model.setAppActive(false)
        model.setAppActive(true)
        clock.advance()
        #expect(marked.ids.isEmpty)
    }

    @Test func nothingIsMarkedWithTheWindowClosed() {
        let (model, clock, marked) = Self.model()
        model.setAppActive(false)
        model.setAppActive(true)
        model.readStateSet()
        clock.advance()
        #expect(marked.ids.isEmpty)
    }
}
