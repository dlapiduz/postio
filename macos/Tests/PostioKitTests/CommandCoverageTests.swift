import PostioFFI
import Testing

@testable import PostioKit

/// The Swift half of `ffi_suite/command_coverage.rs`.
///
/// That sweep asks every registry command whether anything answers it, and it
/// counts `Intercepted` as an answer — this frontend presents a window for
/// those rather than dispatching them. Which makes `Intercepted` load-bearing
/// for the sweep: an id dropped from the Swift list but left in the Rust one
/// is a key that silently does nothing *and* a sweep that says everything is
/// fine.
///
/// So the two copies are held against each other. There are two because Swift
/// matches them by string in a `switch` and needs compile-time constants, and
/// Rust needs them to sweep with; duplication across the boundary is allowed
/// here only because this exists.
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
