import Foundation
import os

/// How long opening a message took, hop by hop -- numbers only.
///
/// "Moving between messages is slow" has four places the time can go: the
/// conversation read, the page the boundary composes, WebKit's load, and
/// whatever waits on the store's write lock in between. This says which,
/// under `log stream --predicate 'subsystem == "dev.postio.Postio"'`.
/// Ids and durations, never content: the logging rule is the same here as in
/// the engine.
public enum ReaderTiming {
    static let log = Logger(subsystem: "dev.postio.Postio", category: "reader")

    /// Milliseconds since `start`.
    public static func ms(since start: ContinuousClock.Instant) -> Int {
        let elapsed = ContinuousClock.now - start
        return Int(elapsed.components.seconds * 1000)
            + Int(elapsed.components.attoseconds / 1_000_000_000_000_000)
    }

    public static func note(_ hop: StaticString, ms: Int, count: Int = 0) {
        log.info("\(hop, privacy: .public) \(ms, privacy: .public) ms (\(count, privacy: .public))")
    }
}
