//! Cross-mailbox sync order: ranks `MailboxRole` by how soon a person looks at it, so the queue is
//! not whatever `LIST` returned (`postio-0d9.6`).
//! Deliberately not `MailboxRole`'s derived `Ord`, which puts Archive right after Inbox.

//! INBOX syncs alone (#1709), keyed on `MailboxRole::Inbox`, never a folder name: its header pass
//! runs with no other mailbox admitted and no background body claimed;
//! then its newest page of bodies (`seed_batch`) before any other mailbox, dropped if not
//! progressing; then the rest in `sync_priority` order.
//! Applies wherever the queue is refilled. A requested body or queued write is never held.

use postio_model::MailboxRole;

/// Where a role sits in the sync queue; lower syncs first. INBOX first, then Flagged, Drafts and
/// Sent, then Archive, Junk and Trash; user folders last.
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
        // View roles are never a real mailbox's role (discovery cannot assign one and storage
        // refuses it), so this arm cannot run.
        MailboxRole::Snoozed | MailboxRole::Outbox => 7,
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
