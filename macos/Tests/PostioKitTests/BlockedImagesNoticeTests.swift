import Testing

@testable import PostioKit

@Suite struct BlockedImagesNoticeTests {
    @Test func aLongAddressLosesItsMiddleRatherThanItsEnds() {
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
