import Foundation
import PostioFFI

/// The rule is `postio_ui::dwell`'s and this asks it rather than restating it: this arms a timer
/// accordingly and decides nothing. A type rather than lines in `Engine` so that "sweeping marks
/// nothing" is testable with a fake scheduler.
@MainActor
public final class DwellClock {
    private let fired: (Int64) -> Void
    /// Injected so a test can drive time rather than wait for it.
    private let schedule: @MainActor (TimeInterval, @escaping () -> Void) -> Cancellable

    private var armed: Cancellable?

    public protocol Cancellable {
        func cancel()
    }

    public init(
        schedule: @escaping @MainActor (TimeInterval, @escaping () -> Void) -> Cancellable =
            DwellClock.afterOnMain,
        fired: @escaping (Int64) -> Void
    ) {
        self.schedule = schedule
        self.fired = fired
    }

    /// Every move cancels: holding `j` re-arms on each row and rests on none, so it marks none.
    /// `nil` cancels and starts nothing, since a clock armed against an unknown message would mark
    /// whichever turned up.
    public func cursorMoved(to message: Int64?) {
        armed?.cancel()
        armed = nil
        guard case let .start(message, milliseconds) = dwellOnCursor(message: message) else {
            return
        }
        armed = schedule(TimeInterval(milliseconds) / 1000) { [weak self] in
            guard let self else { return }
            self.armed = nil
            // Named, not re-read from the cursor: it may have moved on between this firing and
            // running.
            self.fired(message)
        }
    }

    /// For things that make "in front of a person" untrue without moving the cursor (focus lost, an
    /// overlay); they are pushed in rather than watched for.
    public func stop() {
        armed?.cancel()
        armed = nil
    }

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
