import AppKit
import PostioKit

/// The trackpad's back and forward (specs/010-focus-search T073; design
/// §3: "The browser-style back/forward gestures and ⌘[ / ⌘] also move
/// between the inbox and the results").
///
/// The gesture is reported as the command a key would send -- `history_back`
/// or `history_forward` -- and nothing is decided here: whether there is
/// anywhere to go is the controller's (`crates/postio-focus/src/
/// history.rs`), which does nothing with a back it cannot take.
///
/// Two gestures reach a window: a three-finger swipe arrives as one
/// `.swipe` event, and the two-finger "swipe between pages" as horizontal
/// scrolling, which `trackSwipeEvent` turns into one gesture with a
/// direction. Either ends in one command.
@MainActor
public final class HistorySwipe {
    private weak var window: NSWindow?
    private let run: (String) -> Void
    private var monitor: Any?

    public init(window: NSWindow, run: @escaping (String) -> Void) {
        self.window = window
        self.run = run
    }

    /// The command a finished swipe of `amount` sends: right (positive,
    /// as the pages follow the fingers) is back, left is forward, and a
    /// swipe that did not go far is nothing.
    public nonisolated static func command(forAmount amount: CGFloat) -> String? {
        if amount >= 0.5 { return ResultsCommand.historyBack }
        if amount <= -0.5 { return ResultsCommand.historyForward }
        return nil
    }

    /// The command a three-finger `.swipe` with `deltaX` sends: AppKit's
    /// is +1 for a swipe to the left (forward), -1 to the right (back).
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
            // Only a horizontal gesture's start, and only where the system
            // turns scrolling into page swipes.
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
