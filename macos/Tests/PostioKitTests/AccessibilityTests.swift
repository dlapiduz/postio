import PostioFFI
import Testing

@testable import PostioKit

/// What Postio says to a screen reader, and how much it moves.
///
/// The sentence is what can be wrong, so the sentence is what is asserted.
/// `accessibilityLabel` reads back whatever was last set whether or not
/// anything would ever speak it, so a test that set one and read it back would
/// be testing AppKit's property storage — the same trap
/// `docs/archive/engineering-notes.md` records on the GTK side.
@Suite struct AccessibilityTests {
    @Test func everyInterceptedCommandIsInTheRegistry() {
        // The frontend presents a surface for these rather than sending them
        // on, and it matches them by *string*. A literal that no longer names
        // a registered command is a key that silently does nothing — `/` not
        // opening search, and no error anywhere to say why. The registry is
        // the authority, so ask it.
        let known = Set(PostioRegistry.commands.map(\.id))
        for id in Intercepted.all {
            #expect(known.contains(id), "`\(id)` is intercepted and is not a command")
        }
    }
}
