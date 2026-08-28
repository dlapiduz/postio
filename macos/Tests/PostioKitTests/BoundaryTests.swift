import Testing
@testable import PostioKit

/// The boundary, from the Swift side.
///
/// These assert that the *engine* answered, not that Swift compiled. A test
/// that only checked a session could be constructed would pass against a
/// boundary returning nothing at all.
struct BoundaryTests {
    @Test func aSessionOpensOverAnInMemoryStore() throws {
        let session = try PostioSession.open()
        defer { session.shutdown() }
        #expect(session.isOpen)
    }

    @Test func theRegistryCrossesTheBoundary() throws {
        let session = try PostioSession.open()
        defer { session.shutdown() }

        // A number Swift has no way to know on its own: it comes from
        // `postio_core::registry`, which is the point of asking.
        #expect(session.commands.count > 20)

        // And the rows carry what a palette needs, so this cannot pass
        // against a boundary returning empty structs.
        let archive = session.commands.first { $0.id == "archive" }
        let found = try #require(archive, "`archive` is in the registry")
        #expect(!found.title.isEmpty)
        #expect(!found.contexts.isEmpty)
    }

    @Test func shutdownEndsTheDrain() async throws {
        // Swift's drain is `while let event = await nextEvent()`. If the
        // stream never ends, that Task never finishes and the app cannot quit.
        let session = try PostioSession.open()
        session.shutdown()
        let event = await session.nextEvent()
        #expect(event == nil)
    }
}
