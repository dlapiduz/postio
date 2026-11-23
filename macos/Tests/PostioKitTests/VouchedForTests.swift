import PostioFFI
import Testing
@testable import PostioKit

@Suite struct VouchedForTests {
    private func account(_ id: Int64, enabled: Bool = true) -> AccountFfi {
        AccountFfi(
            id: id,
            address: "ada@example.com",
            displayName: "Ada Lovelace",
            initials: "AL",
            isDefault: false,
            enabled: enabled,
            facts: [],
            needsAttention: false,
            repair: .nothing
        )
    }

    @Test func offlineVouchesForNothing() {
        // The "showing local mail" banner uses the same signal; a selection claiming more would
        // contradict it.
        #expect(VouchedFor.accounts([account(1), account(2)], offline: true).isEmpty)
    }

    @Test func onlineVouchesForTheAccountsThatAreSyncing() {
        #expect(VouchedFor.accounts([account(1), account(2)], offline: false) == [1, 2])
    }

    @Test func aDisabledAccountIsNotVouchedFor() {
        // A disabled account is configured, not current: what is on screen for it is whatever was
        // last pulled down.
        let accounts = [account(1), account(2, enabled: false), account(3)]
        #expect(VouchedFor.accounts(accounts, offline: false) == [1, 3])
    }

    @Test func noAccountsIsNoVouching() {
        #expect(VouchedFor.accounts([], offline: false).isEmpty)
    }
}
