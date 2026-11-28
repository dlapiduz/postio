import PostioFFI
import Testing

@testable import PostioKit

@Suite struct AccessibilityTests {
    @Test func everyInterceptedCommandIsInTheRegistry() {
        let known = Set(PostioRegistry.commands.map(\.id))
        for id in Intercepted.all {
            #expect(known.contains(id), "`\(id)` is intercepted and is not a command")
        }
    }
}
