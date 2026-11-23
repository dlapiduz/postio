import Testing
@testable import PostioKit

struct BoundaryTests {
    @Test func theRegistryCrossesTheBoundary() {
        #expect(PostioRegistry.commands.count > 20)
    }

    @Test func aCommandCarriesWhatAPaletteNeeds() throws {
        let archive = PostioRegistry.commands.first { $0.id == "archive" }
        let found = try #require(archive, "`archive` is in the registry")

        #expect(!found.title.isEmpty)
        #expect(!found.defaultBinding.isEmpty)
        #expect(!found.contexts.isEmpty)
    }

    @Test func destructiveCommandsCarryTheirRecovery() {
        // Destructive operations are confirmed or undoable, so `recovery` must survive the
        // crossing.
        for command in PostioRegistry.commands where command.destructive {
            #expect(command.recovery != .none, "\(command.id) is destructive with no recovery")
        }
    }
}
