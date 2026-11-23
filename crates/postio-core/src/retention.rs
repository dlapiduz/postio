//! How long Junk and Trash keep mail before it is emptied (#1932).
//!
//! A Flagged message is never purged, a promise held by `ExpiryRepository::expired_in`.

use chrono::{DateTime, Duration, Utc};
use postio_model::MailboxRole;

/// Days to keep mail per folder kind, `0` for never. Off by default: deleting mail is not something
/// to do unasked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutoEmpty {
    pub junk_days: u32,
    pub trash_days: u32,
}

impl AutoEmpty {
    pub const ROLES: [MailboxRole; 2] = [MailboxRole::Junk, MailboxRole::Trash];

    pub fn is_on(self) -> bool {
        self.junk_days > 0 || self.trash_days > 0
    }

    /// Days to keep mail in a folder of `role`: `None` when it is never
    /// emptied, because the setting is off or the folder is not one of these.
    pub fn days_for(self, role: MailboxRole) -> Option<u32> {
        let days = match role {
            MailboxRole::Junk => self.junk_days,
            MailboxRole::Trash => self.trash_days,
            _ => 0,
        };
        (days > 0).then_some(days)
    }

    pub fn cutoff(self, role: MailboxRole, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let days = self.days_for(role)?;
        Some(now - Duration::days(i64::from(days)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn day(n: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, n, 12, 0, 0).unwrap()
    }

    #[test]
    fn off_by_default_and_for_every_other_folder() {
        let off = AutoEmpty::default();
        assert!(!off.is_on());
        for role in [MailboxRole::Junk, MailboxRole::Trash] {
            assert_eq!(off.cutoff(role, day(30)), None, "{role:?}");
        }
        let on = AutoEmpty {
            junk_days: 7,
            trash_days: 7,
        };
        for role in [
            MailboxRole::Inbox,
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Drafts,
            MailboxRole::Flagged,
            MailboxRole::Regular,
        ] {
            assert_eq!(on.cutoff(role, day(30)), None, "{role:?} is never emptied");
        }
    }

    #[test]
    fn each_folder_keeps_its_own_days() {
        let policy = AutoEmpty {
            junk_days: 7,
            trash_days: 30,
        };
        let now = day(31);
        assert_eq!(policy.cutoff(MailboxRole::Junk, now), Some(day(24)));
        assert_eq!(
            policy.cutoff(MailboxRole::Trash, now),
            Some(now - Duration::days(30))
        );
        assert_eq!(policy.days_for(MailboxRole::Junk), Some(7));
        assert!(policy.is_on());
        let junk_only = AutoEmpty {
            junk_days: 1,
            trash_days: 0,
        };
        assert_eq!(junk_only.days_for(MailboxRole::Trash), None);
        assert!(junk_only.is_on());
    }
}
