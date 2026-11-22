import Network
import Testing
@testable import PostioKit

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
        // `.requiresConnection` is a dial-on-demand VPN: the engine is what brings it up, so
        // reporting offline would stop the attempt that would have worked.
        #expect(Reachability.isOffline(status: .requiresConnection) == false)
    }

    @Test("only an unsatisfied path ever reports offline")
    func offlineIsNarrow() {
        // Stated as a whole so a status added to `NWPath.Status` later must be considered, not
        // default into "offline" and silently mute sync.
        let offline: [NWPath.Status] = [.satisfied, .unsatisfied, .requiresConnection]
            .filter { Reachability.isOffline(status: $0) }
        #expect(offline == [.unsatisfied])
    }
}
