import Foundation
import PostioFFI

/// The sidebar's footer: a state dot and a sentence (canvas screen 25).
///
/// The sentence is `postio_ui::sidebar`'s, so both frontends' footers say the
/// same thing. What is decided here is which two facts to hand it, and that
/// is genuinely this side's: the connection state arrives as events, and when
/// a pass last finished is the newest `lastSyncedAt` across the folders on
/// screen — a store syncs folder by folder, and the footer is about the store.
public enum SidebarFooter {
    /// The line under the folder tree.
    ///
    /// `now` is passed in rather than read, so this is a function of its
    /// arguments and can be asserted without waiting for a clock.
    public static func status(
        mailboxes: [MailboxFfi],
        offline: Bool,
        syncing: Bool,
        now: Date = Date()
    ) -> String {
        // Ranked rather than combined: offline outranks a sync that cannot be
        // running, and a sync in flight outranks a time from the last one.
        let activity: ActivityFfi = offline ? .offline : (syncing ? .syncing : .idle)
        return sidebarStatus(
            activity: activity,
            sinceSeconds: since(mailboxes, now),
            // Whether there is mail here at all, which is what tells "has
            // never synced" apart from "synced, but nobody wrote down when".
            hasMail: mailboxes.contains { $0.total > 0 }
        )
    }

    /// Whether the dot is filled — anything but idle is "something is
    /// happening or wrong", which is what a dot can say and a word cannot.
    public static func isResting(offline: Bool, syncing: Bool) -> Bool {
        !offline && !syncing
    }

    /// How long ago the newest completed pass was, or `nil` if there has
    /// never been one.
    private static func since(_ mailboxes: [MailboxFfi], _ now: Date) -> Int64? {
        guard let newest = mailboxes.compactMap(\.lastSyncedAt).max() else { return nil }
        return Int64(now.timeIntervalSince1970) - newest
    }
}
