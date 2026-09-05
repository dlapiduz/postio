import Foundation

/// The clock that decides a message has been read.
///
/// **The rule is `postio_ui::dwell`'s, not this file's.** The delay comes
/// across the boundary and the arming rule — start on a message, cancel on
/// anything else — is the same one the GTK list has. What is here is the
/// timer, because a timer belongs to whichever run loop owns it.
///
/// Why it is a type and not three lines inside `Engine`: the thing worth
/// asserting is that **sweeping marks nothing**, and that is a statement about
/// a sequence of cursor moves over time. Testing it inside a SwiftUI
/// observable would mean standing up an application; here it needs a fake
/// scheduler and nothing else.
@MainActor
public final class DwellClock {
    /// Called when a message has been in front of somebody long enough.
    private let fired: (Int64) -> Void
    /// How long that is. From the boundary, so both frontends wait the same.
    private let delay: TimeInterval
    /// Schedules `work` after `delay`, and answers something that cancels it.
    ///
    /// Injected so a test can drive time rather than wait for it. The
    /// production value is `DispatchQueue.main.asyncAfter`.
    private let schedule: @MainActor (TimeInterval, @escaping () -> Void) -> Cancellable

    /// What is armed, if anything.
    private var armed: Cancellable?

    /// Something that can be called off.
    public protocol Cancellable {
        func cancel()
    }

    public init(
        delay: TimeInterval,
        schedule: @escaping @MainActor (TimeInterval, @escaping () -> Void) -> Cancellable =
            DwellClock.afterOnMain,
        fired: @escaping (Int64) -> Void
    ) {
        self.delay = delay
        self.schedule = schedule
        self.fired = fired
    }

    /// The cursor moved. Cancel whatever was running; start a clock if there
    /// is a message to start one on.
    ///
    /// Every move cancels, which is the whole rule: holding `j` through a
    /// mailbox re-arms on each row and rests on none, so it marks none of
    /// them. `nil` — the cursor on nothing, or on a row whose page has not
    /// arrived — cancels and starts nothing, because a clock armed against an
    /// unknown message would mark whichever one turned up.
    public func cursorMoved(to message: Int64?) {
        armed?.cancel()
        armed = nil
        guard let message else { return }
        armed = schedule(delay) { [weak self] in
            guard let self else { return }
            self.armed = nil
            // Named, not re-read from the cursor: it may have moved on
            // between this firing and running, and the message that was read
            // is the one the clock was started for.
            self.fired(message)
        }
    }

    /// Stop any clock without starting another.
    ///
    /// For the things that make "in front of a person" untrue without moving
    /// the cursor — the window losing focus, an overlay taking over. Those are
    /// facts about the window rather than about this clock, so they are pushed
    /// in rather than watched for here.
    public func stop() {
        armed?.cancel()
        armed = nil
    }

    /// The production scheduler.
    public static func afterOnMain(
        _ delay: TimeInterval,
        _ work: @escaping () -> Void
    ) -> Cancellable {
        let item = DispatchWorkItem(block: work)
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: item)
        return WorkItem(item: item)
    }

    private struct WorkItem: Cancellable {
        let item: DispatchWorkItem
        func cancel() { item.cancel() }
    }
}
