import PostioFFI

/// Which accounts the unified view can currently vouch for (#811).
///
/// Read at the moment a whole-view selection is *made*, not when a verb
/// runs: `⌘A` in the unified list means "everything in this view", and what
/// that is depends on what Postio can actually see at the moment you press
/// it. Acting later on accounts nothing vouched for would be acting on a
/// guess about somebody's mail.
///
/// **The boundary's default is the empty set**, which is the safe answer and
/// also the wrong one to leave in place: a frontend that never reports this
/// has a `⌘A` that selects nothing at all. This frontend never did.
public enum VouchedFor {
    /// The accounts to report, given what the platform knows.
    ///
    /// Two things, and only two, because they are the two this frontend
    /// actually knows. Offline means nothing can be vouched for — the banner
    /// that says "showing local mail" is drawn from the same signal, and a
    /// selection that claimed more than the banner does would be the
    /// application contradicting itself on one screen. A **disabled** account
    /// is not being synced, so what is on screen for it is whatever was last
    /// pulled down; it is configured, not current.
    ///
    /// Postio has no per-account connection state here. If it grows one,
    /// this is where it goes — and the shape does not change, because this
    /// already answers "which subset", not "yes or no".
    public static func accounts(_ accounts: [AccountFfi], offline: Bool) -> [Int64] {
        guard !offline else { return [] }
        return accounts.filter(\.enabled).map(\.id)
    }
}
