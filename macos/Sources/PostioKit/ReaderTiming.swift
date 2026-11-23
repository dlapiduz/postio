import Foundation
import os

/// Per-hop timings of opening a message, for `log stream --predicate 'subsystem ==
/// "dev.postio.Postio"'`. Ids and durations, never content.
public enum ReaderTiming {
    static let log = Logger(subsystem: "dev.postio.Postio", category: "reader")

    public static func ms(since start: ContinuousClock.Instant) -> Int {
        let elapsed = ContinuousClock.now - start
        return Int(elapsed.components.seconds * 1000)
            + Int(elapsed.components.attoseconds / 1_000_000_000_000_000)
    }

    public static func note(_ hop: StaticString, ms: Int, count: Int = 0) {
        log.info("\(hop, privacy: .public) \(ms, privacy: .public) ms (\(count, privacy: .public))")
    }
}
