import PostioFFI

/// Read when a whole-view selection is made, not when a verb runs (#811). The boundary's default is
/// the empty set, which is safe but leaves `⌘A` selecting nothing, so this frontend must report.
public enum VouchedFor {
    /// Offline vouches for nothing (the "showing local mail" banner uses the same signal). A
    /// disabled account is not synced, so what is on screen for it is stale: configured, not
    /// current.
    public static func accounts(_ accounts: [AccountFfi], offline: Bool) -> [Int64] {
        guard !offline else { return [] }
        return accounts.filter(\.enabled).map(\.id)
    }
}
