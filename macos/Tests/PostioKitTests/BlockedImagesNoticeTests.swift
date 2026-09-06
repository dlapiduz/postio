import Testing

@testable import PostioKit

/// How the blocked-images notice names its sender (#1274).
@Suite struct BlockedImagesNoticeTests {
    @Test func aLongAddressLosesItsMiddleRatherThanItsEnds() {
        // Both ends identify the sender: the local part says which service
        // and the domain says whose it is. A grant is a decision about
        // exactly that, so the half a person judges by must survive.
        let shortened = BlockedImagesNotice.shortened(
            "notices_at_northgate_billing@relay.example.net",
            width: 34
        )

        #expect(shortened.count <= 34)
        #expect(shortened.hasPrefix("notices"))
        #expect(shortened.hasSuffix("relay.example.net"))
        #expect(shortened.contains("…"))
    }

    @Test func anAddressThatFitsIsLeftAlone() {
        #expect(BlockedImagesNotice.shortened("ada@example.com") == "ada@example.com")
    }
}
