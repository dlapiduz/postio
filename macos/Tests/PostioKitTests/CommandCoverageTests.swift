import PostioFFI
import Testing

@testable import PostioKit

@Suite struct CommandCoverageTests {
    @Test func theInterceptedListsAgreeAcrossTheBoundary() {
        let fromRust = Set(interceptedCommands())
        let fromSwift = Set(Intercepted.all)

        #expect(
            fromSwift == fromRust,
            """
            the two Intercepted lists disagree.
              only in Swift: \(fromSwift.subtracting(fromRust).sorted())
              only in Rust:  \(fromRust.subtracting(fromSwift).sorted())
            An id only in Rust is a command the sweep believes this frontend \
            answers and it does not. An id only in Swift is a command the \
            sweep will report as an orphan.
            """
        )
    }

    @Test func everyInterceptedIdNamesARealCommand() {
        let known = Set(PostioRegistry.commands.map(\.id))
        for id in interceptedCommands() {
            #expect(known.contains(id), "`\(id)` is intercepted and is not a command")
        }
    }
}
