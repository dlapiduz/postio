import Foundation
import Network

/// The platform observes; Rust is told: `postio-runtime`'s watcher is NetworkManager over D-Bus,
/// and binding `NWPathMonitor` in Rust would need `unsafe` in a crate that forbids it. This only
/// adds promptness: the engine reconnects with backoff on its own.
public final class Reachability {
    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "dev.postio.reachability")
    private var started = false

    public init() {}

    /// `report` runs on a background queue and may repeat an answer. The boundary absorbs that
    /// (`setOffline` nudges only on a real transition), so no debounce here.
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

    /// Separate from the watching so it can be asserted without a real interface.
    static func isOffline(status: NWPath.Status) -> Bool {
        switch status {
        case .satisfied:
            return false
        case .unsatisfied:
            return true
        case .requiresConnection:
            // "Would be satisfied if something established it" (a dial-on-demand VPN) counts as
            // online: the engine is that something, and reporting offline would stop the attempt
            // that brings the link up.
            return false
        @unknown default:
            // An unknown status assumes a connection: the engine's retry handles being wrong.
            return false
        }
    }
}
