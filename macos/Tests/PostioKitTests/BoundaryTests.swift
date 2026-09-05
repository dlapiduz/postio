import Testing
@testable import PostioKit

/// The boundary, from the Swift side.
///
/// These assert that the *engine* answered, not that Swift compiled. A test
/// that only checked the bindings imported would pass against a boundary
/// returning nothing at all.
///
/// Nothing here opens a session. On macOS that would read the store's key from
/// the login Keychain, and an unsigned test binary has a new code identity on
/// every rebuild — so it would raise a modal prompt on a developer's machine
/// and hang every headless run.
struct BoundaryTests {
    @Test func theRegistryCrossesTheBoundary() {
        // A number Swift has no way to know on its own: it comes from
        // `postio_core::registry`, which is the point of asking.
        #expect(PostioRegistry.commands.count > 20)
    }

    @Test func aCommandCarriesWhatAPaletteNeeds() throws {
        let archive = PostioRegistry.commands.first { $0.id == "archive" }
        let found = try #require(archive, "`archive` is in the registry")

        // Empty strings would mean the rows crossed as hollow structs, which
        // is the failure a count alone cannot see.
        #expect(!found.title.isEmpty)
        #expect(!found.defaultBinding.isEmpty)
        #expect(!found.contexts.isEmpty)
    }

    @Test func destructiveCommandsCarryTheirRecovery() {
        // `PRODUCT.md`: destructive operations are confirmed or undoable. A
        // frontend that cannot see `recovery` cannot honour that, so the fact
        // has to survive the crossing.
        for command in PostioRegistry.commands where command.destructive {
            #expect(command.recovery != .none, "\(command.id) is destructive with no recovery")
        }
    }
}
