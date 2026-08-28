import Network
import Testing
@testable import PostioKit

/// Reading a network path, and what to tell the engine about it.
///
/// `NWPathMonitor` itself is not testable here — it needs a real interface and
/// answers on its own schedule — so the mapping is separated from the watching,
/// which is the same split `MailNotifier` uses. What is left is small and
/// genuinely easy to get wrong: `.requiresConnection` is a *satisfied* status
/// that has no connection yet.
@Suite("Reachability")
struct ReachabilityTests {
    @Test("a satisfied path is online")
    func satisfiedIsOnline() {
        #expect(Reachability.isOffline(status: .satisfied) == false)
    }

    @Test("an unsatisfied path is offline")
    func unsatisfiedIsOffline() {
        #expect(Reachability.isOffline(status: .unsatisfied) == true)
    }

    @Test("a path that only needs establishing is online")
    func requiresConnectionIsOnline() {
        // The case worth pinning. `.requiresConnection` means "would be
        // satisfied if something brought it up" — a dial-on-demand VPN. The
        // engine is that something, so reporting offline would stop it making
        // the attempt that would have worked. Every other mistake this watcher
        // can make costs a second; this one would cost the connection.
        #expect(Reachability.isOffline(status: .requiresConnection) == false)
    }

    @Test("only an unsatisfied path ever reports offline")
    func offlineIsNarrow() {
        // Stated as a whole rather than case by case, so a status added to
        // `NWPath.Status` later has to be considered rather than defaulting
        // into "offline" and silently muting sync.
        let offline: [NWPath.Status] = [.satisfied, .unsatisfied, .requiresConnection]
            .filter { Reachability.isOffline(status: $0) }
        #expect(offline == [.unsatisfied])
    }
}
