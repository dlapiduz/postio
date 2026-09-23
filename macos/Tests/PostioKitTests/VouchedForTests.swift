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
        // The "showing local mail" banner is drawn from the same signal. A
        // selection claiming more than the banner does would be the
        // application contradicting itself on one screen.
        #expect(VouchedFor.accounts([account(1), account(2)], offline: true).isEmpty)
    }

    @Test func onlineVouchesForTheAccountsThatAreSyncing() {
        #expect(VouchedFor.accounts([account(1), account(2)], offline: false) == [1, 2])
    }

    @Test func aDisabledAccountIsNotVouchedFor() {
        // It is configured, not current: nothing has been syncing it, so
        // what is on screen for it is whatever was last pulled down.
        let accounts = [account(1), account(2, enabled: false), account(3)]
        #expect(VouchedFor.accounts(accounts, offline: false) == [1, 3])
    }

    @Test func noAccountsIsNoVouching() {
        #expect(VouchedFor.accounts([], offline: false).isEmpty)
    }
}
