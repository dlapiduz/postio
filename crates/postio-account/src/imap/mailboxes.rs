//! Roles come from the server's `SPECIAL-USE` attribute first; without one (iCloud advertises none)
//! the name is matched against a provider-spelling table. The winner of each role is resolved
//! across the whole listing, so `Projects/Sent` does not win by sorting first.

use std::collections::BTreeSet;

use io_imap::client::ImapClientAsync;
use io_imap::types::core::QuotedChar;
use io_imap::types::flag::FlagNameAttribute;
use io_imap::types::mailbox::{ListMailbox, Mailbox};
use postio_model::MailboxRole;

use crate::backend::{
    BackendError, BackendResult, MailboxFilter, MailboxSummary, resolve_roles_with_known_names,
};
use crate::discovery::preset_for_imap_host;

use super::{ConnectionPool, Dispatch, ImapSession, ListingStrategy, Priority};

/// One connection for the whole listing, so `LIST` and `LSUB` cannot disagree about a folder
/// created between them.
#[tracing::instrument(skip_all, fields(pattern = %filter.pattern, subscribed_only = filter.subscribed_only))]
pub async fn list_mailboxes(
    pool: &ConnectionPool,
    filter: &MailboxFilter,
    priority: Priority,
) -> BackendResult<Vec<MailboxSummary>> {
    let pattern = filter.pattern.clone();
    let subscribed_only = filter.subscribed_only;

    pool.execute(priority, async |session| {
        let strategy =
            Dispatch::new(session.capabilities().clone()).listing_strategy(subscribed_only);
        list_with(session, &pattern, strategy).await
    })
    .await
    .inspect(|mailboxes| {
        // Folder names, not mail content.
        tracing::info!(
            count = mailboxes.len(),
            paths = ?mailboxes.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(),
            "listed the server's folders"
        );
    })
    .inspect_err(|error| tracing::warn!(%error, "cannot list the server's folders"))
    .map(|mailboxes| finish(mailboxes, filter, &pool.settings().host))
}

/// Already existing is success: servers spell that refusal differently, so it asks whether the
/// folder exists afterwards.
/// A failed `SUBSCRIBE` is not fatal: a server refusing it must not make the role unusable.
pub async fn create_mailbox(
    pool: &ConnectionPool,
    path: &str,
    priority: Priority,
) -> BackendResult<()> {
    let path = path.to_owned();

    pool.execute(priority, async |session| {
        let mailbox = mailbox_argument(&path)?;

        if let Err(error) = session.create(mailbox.clone()).await {
            // "Already exists" has no standard spelling; `LIST`ing the one name is cheap.
            let existing = session
                .list(mailbox_argument("")?, list_pattern(&path)?)
                .await
                .map_err(|error| BackendError::Rejected {
                    command: "LIST".to_owned(),
                    reason: error.to_string(),
                })?
                .data;
            if existing.is_empty() {
                return Err(BackendError::Rejected {
                    command: "CREATE".to_owned(),
                    reason: error.to_string(),
                });
            }
        }

        if let Err(error) = session.subscribe(mailbox).await {
            tracing::warn!(%error, "created the folder but could not subscribe to it");
        }
        Ok(())
    })
    .await
    // Ids and outcomes only -- never the folder name a server rejected.
    .inspect(|()| tracing::info!("created a folder"))
    .inspect_err(|error| tracing::warn!(%error, "cannot create a folder"))
}

async fn list_with(
    session: &mut ImapSession,
    pattern: &str,
    strategy: ListingStrategy,
) -> BackendResult<Vec<MailboxSummary>> {
    let reference = mailbox_argument("")?;
    let wildcard = list_pattern(pattern)?;

    let listed = session
        .list(reference.clone(), wildcard.clone())
        .await
        .map_err(|error| BackendError::Rejected {
            command: "LIST".to_owned(),
            reason: error.to_string(),
        })?
        .data;

    let subscribed: Option<BTreeSet<String>> = match strategy {
        ListingStrategy::ListOnly => None,
        ListingStrategy::ListAndLsub => {
            let rows = session.lsub(reference, wildcard).await.map_err(|error| {
                BackendError::Rejected {
                    command: "LSUB".to_owned(),
                    reason: error.to_string(),
                }
            })?;
            Some(
                rows.data
                    .iter()
                    .map(|(name, _, _)| mailbox_name(name))
                    .collect(),
            )
        }
    };

    Ok(listed
        .into_iter()
        .map(|(name, delimiter, attributes)| {
            let path = mailbox_name(&name);
            let mut summary = MailboxSummary::new(
                path.clone(),
                delimiter.as_ref().map(QuotedChar::inner),
                attributes.iter().map(FlagNameAttribute::to_string),
            );
            // With no LSUB, every folder counts as subscribed: the server did
            // not say otherwise, and hiding folders on a guess is worse than
            // showing one too many.
            summary.subscribed = subscribed
                .as_ref()
                .is_none_or(|subscribed| subscribed.contains(&path));
            summary
        })
        .collect())
}

/// `host` is the server this listing came from, not a domain re-derived from the address, so a role
/// tie-break can ask which provider's folder names apply (#959).
fn finish(
    mut mailboxes: Vec<MailboxSummary>,
    filter: &MailboxFilter,
    host: &str,
) -> Vec<MailboxSummary> {
    if filter.subscribed_only {
        mailboxes.retain(|mailbox| mailbox.subscribed);
    }
    let known_names = preset_for_imap_host(host)
        .map(|preset| preset.role_names())
        .unwrap_or_default();
    resolve_roles_with_known_names(&mut mailboxes, &known_names);
    sort_listing(&mut mailboxes);
    mailboxes
}

fn sort_listing(mailboxes: &mut [MailboxSummary]) {
    mailboxes.sort_by(|a, b| {
        let inbox = |mailbox: &MailboxSummary| u8::from(mailbox.role != MailboxRole::Inbox);
        inbox(a)
            .cmp(&inbox(b))
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// `io-imap` has already decoded modified UTF-7.
fn mailbox_name(mailbox: &Mailbox<'_>) -> String {
    match mailbox {
        Mailbox::Inbox => "INBOX".to_owned(),
        Mailbox::Other(other) => {
            let bytes: &[u8] = other.inner().as_ref();
            String::from_utf8_lossy(bytes).into_owned()
        }
    }
}

pub(super) fn mailbox_argument(name: &str) -> BackendResult<Mailbox<'static>> {
    Mailbox::try_from(name.to_owned()).map_err(|error| BackendError::Protocol {
        reason: format!("{name:?} is not a mailbox name IMAP can carry: {error}"),
    })
}

fn list_pattern(pattern: &str) -> BackendResult<ListMailbox<'static>> {
    ListMailbox::try_from(pattern.to_owned()).map_err(|error| BackendError::Protocol {
        reason: format!("{pattern:?} is not a valid LIST pattern: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn summary(path: &str, attributes: &[&str]) -> MailboxSummary {
        MailboxSummary::new(path, Some('/'), attributes.iter().copied())
    }

    fn roles(mailboxes: &[MailboxSummary]) -> Vec<(&str, MailboxRole)> {
        mailboxes
            .iter()
            .map(|mailbox| (mailbox.path.as_str(), mailbox.role))
            .collect()
    }

    #[test]
    fn a_user_folder_does_not_steal_a_role_from_the_real_one() {
        let mut mailboxes = vec![
            summary("Projects/Sent", &[]),
            summary("Sent Messages", &[]),
            summary("INBOX", &[]),
        ];

        resolve_roles_with_known_names(&mut mailboxes, &BTreeMap::new());

        assert_eq!(
            roles(&mailboxes),
            [
                ("Projects/Sent", MailboxRole::Regular),
                ("Sent Messages", MailboxRole::Sent),
                ("INBOX", MailboxRole::Inbox),
            ]
        );
    }

    #[test]
    fn a_server_declared_role_beats_a_shallower_name_match() {
        // A folder literally called "Archive" loses to the one the server
        // marked `\Archive`, however deep that one is.
        let mut mailboxes = vec![
            summary("Archive", &[]),
            summary("[Gmail]/All Mail", &["\\Archive"]),
        ];

        resolve_roles_with_known_names(&mut mailboxes, &BTreeMap::new());

        assert_eq!(
            roles(&mailboxes),
            [
                ("Archive", MailboxRole::Regular),
                ("[Gmail]/All Mail", MailboxRole::Archive),
            ]
        );
    }

    #[test]
    fn two_equally_good_claims_resolve_the_same_way_every_time() {
        let mut first = vec![summary("Junk", &[]), summary("Spam", &[])];
        let mut reversed = vec![summary("Spam", &[]), summary("Junk", &[])];

        resolve_roles_with_known_names(&mut first, &BTreeMap::new());
        resolve_roles_with_known_names(&mut reversed, &BTreeMap::new());

        let junk = |mailboxes: &[MailboxSummary]| {
            mailboxes
                .iter()
                .find(|mailbox| mailbox.role == MailboxRole::Junk)
                .map(|mailbox| mailbox.path.clone())
        };
        assert_eq!(junk(&first), Some("Junk".to_owned()));
        assert_eq!(junk(&first), junk(&reversed));
    }

    #[test]
    fn a_known_provider_name_wins_the_tie_the_alphabet_would_otherwise_win() {
        // iCloud (#501, #943): `Sent` sorts before `Sent Messages`, so the alphabet hands the role
        // to the look-alike.
        let mut mailboxes = vec![summary("Sent", &[]), summary("Sent Messages", &[])];
        let known_names = BTreeMap::from([(MailboxRole::Sent, "Sent Messages".to_owned())]);

        resolve_roles_with_known_names(&mut mailboxes, &known_names);

        assert_eq!(
            roles(&mailboxes),
            [
                ("Sent", MailboxRole::Regular),
                ("Sent Messages", MailboxRole::Sent),
            ]
        );
    }

    /// iCloud's archive folder is `Archive` (60,898 messages); the preset naming `Archives` pointed
    /// the role at a user-made container holding 4 (#1178).
    #[test]
    fn icloud_archive_is_the_folder_the_account_archives_into() {
        let mut mailboxes = vec![
            summary("Archive", &[]),
            summary("Archives", &[]),
            summary("Archives/2024", &[]),
            summary("Archives/2025", &[]),
            summary("Archives/2026", &[]),
            summary("INBOX", &[]),
        ];
        let known_names = crate::discovery::preset_for_domain("icloud.com")
            .expect("iCloud is a shipped preset")
            .role_names();

        resolve_roles_with_known_names(&mut mailboxes, &known_names);

        assert_eq!(
            roles(&mailboxes)
                .into_iter()
                .filter(|(_, role)| *role == MailboxRole::Archive)
                .collect::<Vec<_>>(),
            [("Archive", MailboxRole::Archive)],
            "the role has to land on the folder the account's archived mail \
             is actually in -- pointing it at `Archives` files every `a` into \
             a folder holding 4 messages while 60,898 sit next to it"
        );
    }

    #[test]
    fn a_known_name_absent_from_the_listing_falls_back_to_todays_behaviour() {
        // A preset name the server never lists must not suppress the role, only decline to settle
        // it (#959).
        let mut with_hint = vec![summary("Archive", &[]), summary("Nested/Archive", &[])];
        let mut without_hint = with_hint.clone();
        let known_names = BTreeMap::from([(MailboxRole::Archive, "Archives".to_owned())]);

        resolve_roles_with_known_names(&mut with_hint, &known_names);
        resolve_roles_with_known_names(&mut without_hint, &BTreeMap::new());

        assert_eq!(roles(&with_hint), roles(&without_hint));
        assert_eq!(
            roles(&with_hint),
            [
                ("Archive", MailboxRole::Archive),
                ("Nested/Archive", MailboxRole::Regular),
            ],
            "the shallowest match should still win when the known name matched nothing"
        );
    }

    #[test]
    fn a_server_declared_role_still_beats_a_known_name() {
        // The server's own SPECIAL-USE is still the first word, even over a
        // folder that happens to be spelled exactly like the preset's own
        // name for a different, plainly-named folder.
        let mut mailboxes = vec![
            summary("Sent Messages", &[]),
            summary("[Gmail]/Sent Mail", &["\\Sent"]),
        ];
        let known_names = BTreeMap::from([(MailboxRole::Sent, "Sent Messages".to_owned())]);

        resolve_roles_with_known_names(&mut mailboxes, &known_names);

        assert_eq!(
            roles(&mailboxes),
            [
                ("Sent Messages", MailboxRole::Regular),
                ("[Gmail]/Sent Mail", MailboxRole::Sent),
            ]
        );
    }

    #[test]
    fn the_inbox_is_never_contested() {
        let mut mailboxes = vec![summary("INBOX", &[]), summary("INBOX/Sub", &[])];

        resolve_roles_with_known_names(&mut mailboxes, &BTreeMap::new());

        assert_eq!(mailboxes[0].role, MailboxRole::Inbox);
    }

    #[test]
    fn the_listing_puts_the_inbox_first_and_is_otherwise_stable() {
        let mut mailboxes = vec![
            summary("Zebra", &[]),
            summary("INBOX", &[]),
            summary("archive", &[]),
        ];

        sort_listing(&mut mailboxes);

        let paths: Vec<&str> = mailboxes.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, ["INBOX", "archive", "Zebra"]);
    }
}
