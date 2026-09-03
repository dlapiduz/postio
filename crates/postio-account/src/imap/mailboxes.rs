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

use std::collections::{BTreeMap, BTreeSet};

use io_imap::client::ImapClientAsync;
use io_imap::types::core::QuotedChar;
use io_imap::types::flag::FlagNameAttribute;
use io_imap::types::mailbox::{ListMailbox, Mailbox};
use postio_model::MailboxRole;

use crate::backend::{BackendError, BackendResult, MailboxFilter, MailboxSummary};

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
    .map(|mailboxes| finish(mailboxes, filter))
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
fn finish(mut mailboxes: Vec<MailboxSummary>, filter: &MailboxFilter) -> Vec<MailboxSummary> {
    if filter.subscribed_only {
        mailboxes.retain(|mailbox| mailbox.subscribed);
    }
    resolve_roles(&mut mailboxes);
    sort_listing(&mut mailboxes);
    mailboxes
}

/// Settles which folder actually holds each special-use role.
///
/// Two rules, in order:
///
/// 1. **The server wins.** A folder the server marked `\Sent` is the sent
///    folder, whatever it is called and whatever else looks like one.
/// 2. **Otherwise, the shallowest name match wins.** `Sent Messages` at the
///    top level beats `Projects/Sent`, and the loser goes back to being an
///    ordinary folder. Ties break alphabetically so a listing resolves the
///    same way every time.
///
/// [`MailboxRole::Inbox`] is exempt: `INBOX` is reserved by RFC 3501 and
/// cannot be contested.
fn resolve_roles(mailboxes: &mut [MailboxSummary]) {
    let mut winners: BTreeMap<MailboxRole, usize> = BTreeMap::new();

    for (index, mailbox) in mailboxes.iter().enumerate() {
        let role = mailbox.role;
        if !role.is_special() || role == MailboxRole::Inbox {
            continue;
        }
        match winners.get(&role) {
            None => {
                winners.insert(role, index);
            }
            Some(&held) => {
                if beats(mailbox, &mailboxes[held]) {
                    winners.insert(role, index);
                }
            }
        }
    }

    for (index, mailbox) in mailboxes.iter_mut().enumerate() {
        let role = mailbox.role;
        if !role.is_special() || role == MailboxRole::Inbox {
            continue;
        }
        if winners.get(&role) != Some(&index) {
            mailbox.role = MailboxRole::Regular;
        }
    }
}

/// Whether `candidate` has a better claim to its role than `held`.
fn beats(candidate: &MailboxSummary, held: &MailboxSummary) -> bool {
    let candidate_declared = declares_role(candidate);
    let held_declared = declares_role(held);
    if candidate_declared != held_declared {
        return candidate_declared;
    }
    match candidate.depth().cmp(&held.depth()) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => candidate.path < held.path,
    }
}

/// Whether the server itself named this folder's role.
fn declares_role(mailbox: &MailboxSummary) -> bool {
    mailbox
        .attributes
        .iter()
        .any(|attribute| MailboxRole::from_special_use(attribute).is_some())
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

        resolve_roles(&mut mailboxes);

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

        resolve_roles(&mut mailboxes);

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

        resolve_roles(&mut first);
        resolve_roles(&mut reversed);

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
    fn the_inbox_is_never_contested() {
        let mut mailboxes = vec![summary("INBOX", &[]), summary("INBOX/Sub", &[])];

        resolve_roles(&mut mailboxes);

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
