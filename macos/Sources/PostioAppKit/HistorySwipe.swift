import AppKit
import PostioKit

/// Reported as the command a key would send; whether there is anywhere to go is the controller's.
/// A three-finger swipe arrives as one `.swipe` event, the two-finger one as horizontal scrolling,
/// which `trackSwipeEvent` turns into one gesture.
@MainActor
public final class HistorySwipe {
    private weak var window: NSWindow?
    private let run: (String) -> Void
    private var monitor: Any?

    public init(window: NSWindow, run: @escaping (String) -> Void) {
        self.window = window
        self.run = run
    }

    public nonisolated static func command(forAmount amount: CGFloat) -> String? {
        if amount >= 0.5 { return ResultsCommand.historyBack }
        if amount <= -0.5 { return ResultsCommand.historyForward }
        return nil
    }

    /// AppKit's deltaX is +1 for a swipe to the left (forward), -1 to the right (back).
    public nonisolated static func command(forSwipeDeltaX deltaX: CGFloat) -> String? {
        if deltaX > 0 { return ResultsCommand.historyForward }
        if deltaX < 0 { return ResultsCommand.historyBack }
        return nil
    }

    public func start() {
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.swipe, .scrollWheel]) { [weak self] event in
            guard let self, let window = self.window, event.window === window else { return event }
            return self.handle(event)
        }
    }

    public func stop() {
        if let monitor { NSEvent.removeMonitor(monitor) }
        monitor = nil
    }

    private func handle(_ event: NSEvent) -> NSEvent? {
        switch event.type {
        case .swipe:
            guard let command = Self.command(forSwipeDeltaX: event.deltaX) else { return event }
            run(command)
            return nil
        case .scrollWheel:
            guard event.phase == .began, NSEvent.isSwipeTrackingFromScrollEventsEnabled,
                  abs(event.scrollingDeltaX) > abs(event.scrollingDeltaY)
            else { return event }
            var said = false
            event.trackSwipeEvent(
                options: [.lockDirection, .clampGestureAmount],
                dampenAmountThresholdMin: -1, max: 1
            ) { [weak self] amount, phase, complete, stop in
                MainActor.assumeIsolated {
                    if phase == .ended, !said, let command = Self.command(forAmount: amount) {
                        said = true
                        self?.run(command)
                    }
                    if complete { stop.pointee = true }
                }
            }
            return nil
        default:
            return event
        }
    }
}
