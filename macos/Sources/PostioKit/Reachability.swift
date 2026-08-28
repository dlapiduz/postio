import Foundation
import Network

/// Watching the network path, and telling the engine about it.
///
/// **The platform observes; Rust is told.** `postio-runtime`'s watcher is
/// NetworkManager over D-Bus and returns immediately here, leaving the state
/// unknown; binding `NWPathMonitor` in Rust would mean `unsafe` in a crate
/// that forbids it, for a signal `Network` hands over in a few lines.
///
/// This does not make anything work that did not. The engine reconnects with
/// backoff on its own and runs with no signal at all — pull this out and the
/// application still syncs, only slower to notice. What it buys is
/// *promptness*: waking a laptop reconnects now rather than at the next
/// backoff step, and the reader can say "offline" instead of "still
/// downloading" for a body that is not coming. That distinction is the whole
/// reason those are separate absence states.
public final class Reachability {
    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "dev.postio.reachability")
    private var started = false

    public init() {}

    /// Report every change to `report`, until this object goes away.
    ///
    /// `report` is called on a background queue and may be called with the
    /// same answer twice — an interface changing while the path stays
    /// satisfied is a fresh callback saying nothing new. The boundary
    /// deliberately absorbs that: `setOffline` nudges only on a real
    /// transition, so no debounce is needed here and a second one would only
    /// add a place for the two to disagree.
    public func start(_ report: @escaping @Sendable (Bool) -> Void) {
        guard !started else { return }
        started = true
        monitor.pathUpdateHandler = { path in
            report(Self.isOffline(status: path.status))
        }
        monitor.start(queue: queue)
    }

    public func stop() {
        monitor.cancel()
    }

    /// Whether a path means "no connection right now".
    ///
    /// Separated from the watching so it can be asserted: `NWPathMonitor`
    /// needs a real interface and answers on its own schedule, and this is
    /// the part with a decision in it.
    static func isOffline(status: NWPath.Status) -> Bool {
        switch status {
        case .satisfied:
            return false
        case .unsatisfied:
            return true
        case .requiresConnection:
            // "Would be satisfied if something established it" — a
            // dial-on-demand VPN, typically. **Online**, because the engine is
            // exactly that something: it will try, and trying is what brings
            // the link up. Reporting offline would stop it making the attempt
            // that would have worked — every other mistake this watcher can
            // make costs a second, and this one would cost the connection.
            return false
        @unknown default:
            // A status this build has not heard of. Assuming a connection is
            // the safe default: the engine's own retry handles being wrong,
            // while claiming offline is a sentence the reader renders at
            // somebody.
            return false
        }
    }
}
