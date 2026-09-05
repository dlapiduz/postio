//! Finding out what folders an account has, and what each one is *for*.
//!
//! Routing archive, trash, junk and sent by name does not work: iCloud calls
//! them `Sent Messages` and `Deleted Messages`, Gmail hides them under
//! `[Gmail]/`, Exchange says `Sent Items`. Routing by RFC 6154 `SPECIAL-USE`
//! attributes alone does not work either: iCloud advertises none.
//!
//! So both are used. The server's attribute always wins; where there is none,
//! the folder name is matched against a table that knows the provider
//! spellings, and — because a name can match by accident — the winner of each
//! role is resolved across the whole listing rather than per row. A user's
//! `Projects/Sent` does not become the account's Sent folder just because it
//! sorted first.
//!
//! Persisting the result is the repository's job. What comes out of here is a
//! [`MailboxSummary`] per folder, with its role, its hierarchy and whether the
//! account is subscribed to it.

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

/// Lists the account's folders and resolves each one's role.
///
/// One connection is used for the whole listing, so `LIST` and the `LSUB` that
/// tells us what is subscribed cannot disagree about a folder that was created
/// between them.
#[tracing::instrument(skip_all, fields(pattern = %filter.pattern, subscribed_only = filter.subscribed_only))]
pub async fn list_mailboxes(
    pool: &ConnectionPool,
    filter: &MailboxFilter,
    priority: Priority,
) -> BackendResult<Vec<MailboxSummary>> {
    let pattern = filter.pattern.clone();
    let subscribed_only = filter.subscribed_only;

    pool.execute(priority, async |session| {
        // The strategy is chosen from *this* session's capability set rather
        // than a pool-level cache, so the very first listing of an account
        // gets the same decision as every later one.
        let strategy =
            Dispatch::new(session.capabilities().clone()).listing_strategy(subscribed_only);
        list_with(session, &pattern, strategy).await
    })
    .await
    .inspect(|mailboxes| {
        // Folder *names*: the containers, not the mail in them. This is the
        // line that says whether a server answered a LIST at all.
        tracing::info!(
            count = mailboxes.len(),
            paths = ?mailboxes.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(),
            "listed the server's folders"
        );
    })
    .inspect_err(|error| tracing::warn!(%error, "cannot list the server's folders"))
    .map(|mailboxes| finish(mailboxes, filter, &pool.settings().host))
}

/// Issues the listing commands against one open session.
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
        })?;

    let subscribed: Option<BTreeSet<String>> = match strategy {
        ListingStrategy::ListOnly => None,
        ListingStrategy::ListAndLsub => {
            let rows = session.lsub(reference, wildcard).await.map_err(|error| {
                BackendError::Rejected {
                    command: "LSUB".to_owned(),
                    reason: error.to_string(),
                }
            })?;
            Some(rows.iter().map(|(name, _, _)| mailbox_name(name)).collect())
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

/// Applies the filter, settles contested roles, and puts the listing in a
/// stable order.
///
/// `host` is the server this listing came from -- `pool.settings().host`,
/// never a domain re-derived from the address -- so a role tie-break can ask
/// which provider's own folder names apply (#959), the same host the
/// account is already connected to rather than a fresh discovery lookup.
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

/// Orders a listing predictably: `INBOX`, then the rest by path.
///
/// A stable order, not a display order — the sidebar decides how folders are
/// presented. This exists so that two listings of an unchanged account compare
/// equal.
fn sort_listing(mailboxes: &mut [MailboxSummary]) {
    mailboxes.sort_by(|a, b| {
        let inbox = |mailbox: &MailboxSummary| u8::from(mailbox.role != MailboxRole::Inbox);
        inbox(a)
            .cmp(&inbox(b))
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// The mailbox name as text, `INBOX` included.
///
/// `io-imap` has already decoded modified UTF-7, so these are ordinary
/// unicode names by the time they reach us.
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
        // iCloud's own account (#501, #943): `Sent` sorts before
        // `Sent Messages`, so the alphabet alone hands the role to the
        // look-alike another client left behind. The provider's own name
        // settles it instead.
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

    /// iCloud's archive folder is `Archive`, and the preset naming it
    /// `Archives` points the role at a folder the user made (#1178).
    ///
    /// Measured on the live account #959 and #501 were both reported from:
    /// `Archive` holds 60,898 messages and has no children; `Archives` holds
    /// 4, with `2024`, `2025` and `2026` under it holding 42 between them.
    /// The second is somebody's filing scheme, and #959's own criterion for
    /// telling the twins apart — "which twin has mail in it" — picks the
    /// first by a factor of 1,300.
    ///
    /// This is the one row of the iCloud preset that inverted a default that
    /// was already right: `Sent`/`Sent Messages` and `Trash`/`Deleted
    /// Messages` need the preset, because the alphabet picks the wrong twin
    /// there. `Archive` sorts before `Archives`, so the alphabet was correct
    /// and the preset overrode it.
    #[test]
    fn icloud_archive_is_the_folder_the_account_archives_into() {
        // The shape of the real account: no SPECIAL-USE anywhere, the
        // provider's own archive beside a user-made container of years.
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
        // A preset name the server never lists -- nothing here should
        // suppress the role, only decline to help settle it. #959's third
        // acceptance criterion. The name is deliberately not iCloud's own
        // (which is "Archive", #1178): this case is about a *missing* one.
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
