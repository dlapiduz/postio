//! Cross-mailbox sync order: which folder gets synced first.
//!
//! # Why this exists
//!
//! [`initial`](crate::initial) already orders *within* one mailbox — newest
//! `UID` first — so the first screenful of a huge folder is visible fast.
//! Nothing ordered *across* mailboxes: the queue the engine drained was built
//! from whatever `LIST` happened to return, and a 37,699-message Archive could
//! start syncing while INBOX had not been touched at all (`postio-0d9.6`). This
//! module is the missing half: it ranks [`MailboxRole`] by how soon a person is
//! likely to look at it, so the sync queue is built in that order rather than
//! letting it emerge from discovery.
//!
//! Deliberately not [`MailboxRole`]'s derived `Ord` — that ordering exists so
//! roles are a well-formed key elsewhere (storage round-trips, `BTreeMap`s) and
//! it runs `Inbox, Archive, Sent, ...`. Archive right after Inbox is wrong for
//! *this* purpose: it is exactly the huge folder nobody is waiting on.

use postio_model::MailboxRole;

/// Where a role sits in the sync queue. Lower syncs first.
///
/// INBOX is unconditionally first. Then the folders a person reads next —
/// Flagged (their own follow-up list), Drafts (work in progress) and Sent
/// (what they just wrote) — sync before anything spends time on Archive, Junk
/// or Trash. Regular user folders are unranked among themselves and sync last.
pub fn sync_priority(role: MailboxRole) -> u8 {
    match role {
        MailboxRole::Inbox => 0,
        MailboxRole::Flagged => 1,
        MailboxRole::Drafts => 2,
        MailboxRole::Sent => 3,
        MailboxRole::Archive => 4,
        MailboxRole::Junk => 5,
        MailboxRole::Trash => 6,
        MailboxRole::Regular => 7,
        // Never a real mailbox's role: no `SPECIAL-USE` attribute names it,
        // so discovery can never assign it and this arm can never actually
        // run. Ranked with `Regular` rather than matched some other way, so
        // an exhaustive match stays exhaustive without inventing meaning
        // this queue does not need.
        MailboxRole::Snoozed => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_sorts_before_every_other_role() {
        for role in [
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Drafts,
            MailboxRole::Trash,
            MailboxRole::Junk,
            MailboxRole::Flagged,
            MailboxRole::Regular,
        ] {
            assert!(sync_priority(MailboxRole::Inbox) < sync_priority(role));
        }
    }

    #[test]
    fn a_shuffled_role_list_sorts_into_the_read_order() {
        let mut roles = [
            MailboxRole::Regular,
            MailboxRole::Trash,
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Junk,
            MailboxRole::Drafts,
            MailboxRole::Flagged,
            MailboxRole::Inbox,
        ];
        roles.sort_by_key(|role| sync_priority(*role));
        assert_eq!(
            roles,
            [
                MailboxRole::Inbox,
                MailboxRole::Flagged,
                MailboxRole::Drafts,
                MailboxRole::Sent,
                MailboxRole::Archive,
                MailboxRole::Junk,
                MailboxRole::Trash,
                MailboxRole::Regular,
            ]
        );
    }

    #[test]
    fn large_archive_never_outranks_a_folder_someone_reads() {
        // The bug this module exists to fix: a 37,699-message Archive queued
        // ahead of INBOX. Every role a person actually reads must rank below
        // Archive.
        for reads_first in [
            MailboxRole::Inbox,
            MailboxRole::Flagged,
            MailboxRole::Drafts,
            MailboxRole::Sent,
        ] {
            assert!(sync_priority(reads_first) < sync_priority(MailboxRole::Archive));
        }
    }
}
