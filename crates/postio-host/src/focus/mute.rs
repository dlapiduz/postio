//! A muted thread's new replies are archived on arrival (spec 013 US5,
//! FR-012, research R4).
//!
//! The first thing the filing stage does with an arrival, before any rule of
//! Focus's own and before a snooze is ended: if the message joined a thread
//! the person muted, it is archived the way any archive archives (an
//! ordinary move the server hears of), the thread's `replies_since` goes up,
//! and nothing else runs on it. It is **not** a Filtered decision: Filtered
//! is "Focus judged this mail", and mute is the person's own instruction.
//! The message stays in the store and in the search index; only the inbox
//! loses it.
//!
//! **Exempt:** a message addressed to the person alone, one of their own
//! addresses the only `To` and no `Cc`, arrives in the inbox as usual and the
//! thread stays muted: somebody wrote to you.
//!
//! Cost: one statement for the whole batch when it holds any mail received
//! into an inbox ([`ThreadRepository::muted`]), none otherwise; the move is
//! written only for the replies it archives.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use postio_model::{AccountId, EmailAddress, MailboxId, MailboxRole, MessageId, ThreadId};
use postio_storage::Connection;
use postio_storage::actions::{self, Relocation};
use postio_storage::repository::{IdentityRepository, MailboxRepository, ThreadRepository};
use postio_sync::{FiledMessage, SyncError};

/// Archives the replies in `filed` that joined a muted thread, and answers
/// the arrivals that go on to be filed as usual.
///
/// A failure is the person's mail staying in the inbox: it is logged by
/// count and the whole batch goes on.
pub(crate) async fn archive_muted<'a>(
    transaction: &Connection,
    filed: &[FiledMessage<'a>],
) -> Result<Vec<FiledMessage<'a>>, SyncError> {
    match try_archive(transaction, filed).await {
        Ok(rest) => Ok(rest),
        Err(error) => {
            tracing::warn!(%error, "muted replies could not be archived: {error}");
            Ok(filed.to_vec())
        }
    }
}

async fn try_archive<'a>(
    transaction: &Connection,
    filed: &[FiledMessage<'a>],
) -> Result<Vec<FiledMessage<'a>>, postio_storage::Error> {
    let mut threads: Vec<ThreadId> = filed
        .iter()
        .filter(|message| message.role == MailboxRole::Inbox)
        .filter_map(|message| message.thread)
        .collect();
    if threads.is_empty() {
        return Ok(filed.to_vec());
    }
    threads.sort_unstable();
    threads.dedup();
    let repository = ThreadRepository::new(transaction);
    let muted = repository.muted(&threads).await?;
    if muted.is_empty() {
        return Ok(filed.to_vec());
    }

    let own: BTreeSet<String> = IdentityRepository::new(transaction)
        .own_addresses()
        .await?
        .iter()
        .map(EmailAddress::normalized)
        .collect();
    let mut archives: BTreeMap<AccountId, Option<MailboxId>> = BTreeMap::new();
    let mut leaving: BTreeMap<(AccountId, MailboxId), BTreeMap<MailboxId, Vec<MessageId>>> =
        BTreeMap::new();
    let mut counted: BTreeMap<ThreadId, u32> = BTreeMap::new();
    let mut rest = Vec::with_capacity(filed.len());
    for message in filed {
        let thread = message.thread.filter(|thread| muted.contains_key(thread));
        let (Some(thread), MailboxRole::Inbox) = (thread, message.role) else {
            rest.push(*message);
            continue;
        };
        if addressed_to_you_alone(message.message, &own) {
            rest.push(*message);
            continue;
        }
        let account = message.message.account_id;
        let archive = match archives.get(&account) {
            Some(archive) => *archive,
            None => {
                let archive = MailboxRepository::new(transaction)
                    .by_role(account, MailboxRole::Archive)
                    .await?
                    .map(|mailbox| mailbox.id);
                archives.insert(account, archive);
                archive
            }
        };
        let Some(archive) = archive.filter(|archive| *archive != message.message.mailbox_id) else {
            rest.push(*message);
            continue;
        };
        leaving
            .entry((account, archive))
            .or_default()
            .entry(message.message.mailbox_id)
            .or_default()
            .push(message.message.id);
        *counted.entry(thread).or_insert(0) += 1;
    }
    let now = Utc::now();
    for ((account, archive), by_source) in &leaving {
        actions::relocate(
            transaction,
            *account,
            by_source,
            *archive,
            Relocation::Move,
            now,
        )
        .await?;
    }
    for (thread, replies) in &counted {
        repository.note_muted_replies(*thread, *replies).await?;
    }
    tracing::debug!(
        archived = counted.values().sum::<u32>(),
        threads = counted.len(),
        "replies in muted threads were archived"
    );
    Ok(rest)
}

/// Whether `message` is addressed to the person alone: one of `own` is its
/// only `To` address, and it has no `Cc`.
fn addressed_to_you_alone(message: &postio_model::Message, own: &BTreeSet<String>) -> bool {
    message.cc.is_empty() && message.to.len() == 1 && own.contains(&message.to[0].normalized())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::{DateTime, Utc};
    use postio_model::{Flag, Message, RfcMessageId};
    use postio_storage::repository::{
        FilterDecisionRepository, MessageRepository, OperationQueueRepository, ThreadingRepository,
    };
    use postio_storage::test_support::{self, counting};
    use postio_sync::{FilingEffects, FilingPass};

    use super::super::reply_wakes::ReplyWakes;
    use super::*;

    /// A filing pass that remembers what it was handed.
    #[derive(Debug, Default)]
    struct Seen(Mutex<Vec<MessageId>>);

    #[async_trait::async_trait]
    impl FilingPass for Seen {
        async fn file(
            &self,
            _transaction: &Connection,
            filed: &[FiledMessage<'_>],
        ) -> Result<FilingEffects, SyncError> {
            self.0
                .lock()
                .expect("never poisoned")
                .extend(filed.iter().map(|message| message.message.id));
            Ok(FilingEffects::default())
        }
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_800_000_000 + minutes * 60, 0).expect("a time")
    }

    struct Scene {
        _database: postio_storage::Store,
        connection: postio_storage::Checkout,
        account: AccountId,
        inbox: MailboxId,
        archive: Option<MailboxId>,
        thread: ThreadId,
        seen: Arc<Seen>,
        pass: Arc<dyn FilingPass>,
    }

    /// A conversation of one in the inbox, muted.
    async fn a_muted_thread(with_archive: bool) -> Scene {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let archive = if with_archive {
            Some(
                test_support::mailbox(&connection, &account, "Archive")
                    .await
                    .id,
            )
        } else {
            None
        };
        let seen = Arc::new(Seen::default());
        let mut scene = Scene {
            _database: database,
            connection,
            account: account.id,
            inbox,
            archive,
            thread: ThreadId::new(0),
            pass: ReplyWakes::wrapping(seen.clone()),
            seen,
        };
        let (_, thread) = scene.arrival("m1", &[], &["ada@example.com"], &[], 1).await;
        scene.thread = thread;
        ThreadRepository::new(&scene.connection)
            .mute(thread)
            .await
            .expect("mute");
        scene
    }

    impl Scene {
        /// A message stored and threaded as sync does, addressed as given.
        async fn arrival(
            &self,
            rfc: &str,
            references: &[&str],
            to: &[&str],
            cc: &[&str],
            minutes: i64,
        ) -> (Message, ThreadId) {
            let mut message = Message::new(self.account, self.inbox, at(minutes));
            message.rfc_message_id = Some(RfcMessageId::new(format!("<{rfc}@example.com>")));
            message.references = references
                .iter()
                .map(|id| RfcMessageId::new(format!("<{id}@example.com>")))
                .collect();
            message.in_reply_to = message.references.last().cloned();
            message.subject = Some(if rfc.starts_with('x') {
                "Re: Something else".to_owned()
            } else {
                "Re: Harbor API".to_owned()
            });
            message.flags = [Flag::Seen].into_iter().collect();
            message.to = to
                .iter()
                .map(|address| EmailAddress::new(None::<String>, *address))
                .collect();
            message.cc = cc
                .iter()
                .map(|address| EmailAddress::new(None::<String>, *address))
                .collect();
            MessageRepository::new(&self.connection)
                .create(&mut message)
                .await
                .expect("a message");
            let thread = ThreadingRepository::new(&self.connection, self.account)
                .thread(&message)
                .await
                .expect("threaded")
                .thread_id;
            (message, thread)
        }

        async fn files(&self, message: &Message, thread: ThreadId, role: MailboxRole) {
            let filed = [FiledMessage {
                message,
                thread: Some(thread),
                role,
            }];
            self.pass
                .file(&self.connection, &filed)
                .await
                .expect("filed");
        }

        async fn mailbox_of(&self, message: &Message) -> MailboxId {
            MessageRepository::new(&self.connection)
                .get(message.id)
                .await
                .expect("a read")
                .expect("still stored")
                .mailbox_id
        }

        async fn replies_since(&self) -> Option<u32> {
            ThreadRepository::new(&self.connection)
                .muted(&[self.thread])
                .await
                .expect("a read")
                .get(&self.thread)
                .copied()
        }

        fn handed_on(&self, message: &Message) -> bool {
            self.seen
                .0
                .lock()
                .expect("never poisoned")
                .contains(&message.id)
        }
    }

    #[tokio::test]
    async fn a_reply_in_a_muted_thread_is_archived_counted_and_judged_by_nothing_else() {
        let scene = a_muted_thread(true).await;
        let (reply, thread) = scene
            .arrival(
                "m2",
                &["m1"],
                &["ada@example.com", "ben@example.com"],
                &[],
                2,
            )
            .await;
        assert_eq!(thread, scene.thread, "the fixture joins the muted thread");

        scene.files(&reply, thread, MailboxRole::Inbox).await;

        let archive = scene.archive.expect("an archive");
        assert_eq!(scene.mailbox_of(&reply).await, archive, "out of the inbox");
        assert_eq!(scene.replies_since().await, Some(1));
        let queued = OperationQueueRepository::new(&scene.connection)
            .pending(scene.account, Utc::now())
            .await
            .expect("a read");
        assert!(
            queued.iter().any(|row| matches!(
                row.operation,
                postio_model::Operation::Move { from, to } if from == scene.inbox && to == archive
            )),
            "the server is told, as for any archive: {queued:?}"
        );
        assert!(
            FilterDecisionRepository::new(&scene.connection)
                .get(reply.id)
                .await
                .expect("a read")
                .is_none(),
            "mute is the person's word, not a Filtered decision"
        );
        assert!(!scene.handed_on(&reply), "no other filing rule runs on it");
        // It is still in the store (searchable), not deleted.
        let stored = MessageRepository::new(&scene.connection)
            .get(reply.id)
            .await
            .expect("a read")
            .expect("stored");
        assert!(!stored.sync.deleted_locally);
    }

    #[tokio::test]
    async fn a_reply_addressed_to_you_alone_arrives_in_the_inbox_and_the_thread_stays_muted() {
        let scene = a_muted_thread(true).await;
        let (reply, thread) = scene
            .arrival("m2", &["m1"], &["Test@Example.com"], &[], 2)
            .await;

        scene.files(&reply, thread, MailboxRole::Inbox).await;

        assert_eq!(
            scene.mailbox_of(&reply).await,
            scene.inbox,
            "in the inbox as usual"
        );
        assert!(scene.handed_on(&reply), "filed as any arrival is");
        assert_eq!(
            scene.replies_since().await,
            Some(0),
            "still muted, nothing counted"
        );
    }

    #[tokio::test]
    async fn a_cc_or_a_second_recipient_takes_the_exemption_away() {
        for (to, cc) in [
            (vec!["test@example.com", "ben@example.com"], vec![]),
            (vec!["test@example.com"], vec!["ben@example.com"]),
            (vec!["ben@example.com"], vec!["test@example.com"]),
            (vec![], vec![]),
        ] {
            let scene = a_muted_thread(true).await;
            let (reply, thread) = scene.arrival("m2", &["m1"], &to, &cc, 2).await;

            scene.files(&reply, thread, MailboxRole::Inbox).await;

            assert_eq!(
                scene.mailbox_of(&reply).await,
                scene.archive.expect("an archive"),
                "to {to:?}, cc {cc:?}: archived"
            );
            assert_eq!(scene.replies_since().await, Some(1), "to {to:?}, cc {cc:?}");
        }
    }

    #[tokio::test]
    async fn a_thread_that_is_not_muted_and_mail_not_received_are_left_alone() {
        let scene = a_muted_thread(true).await;
        ThreadRepository::new(&scene.connection)
            .unmute(scene.thread)
            .await
            .expect("unmute");
        let (reply, thread) = scene
            .arrival("m2", &["m1"], &["ben@example.com"], &[], 2)
            .await;
        scene.files(&reply, thread, MailboxRole::Inbox).await;
        assert_eq!(scene.mailbox_of(&reply).await, scene.inbox, "not muted");
        assert!(scene.handed_on(&reply));

        // Muted again, and the account's own reply in Sent is not "received".
        ThreadRepository::new(&scene.connection)
            .mute(scene.thread)
            .await
            .expect("mute");
        let (sent, thread) = scene
            .arrival("m3", &["m2"], &["ben@example.com"], &[], 3)
            .await;
        scene.files(&sent, thread, MailboxRole::Sent).await;
        assert_eq!(scene.mailbox_of(&sent).await, scene.inbox, "untouched");
        assert!(scene.handed_on(&sent));
        assert_eq!(scene.replies_since().await, Some(0));
    }

    #[tokio::test]
    async fn an_account_with_no_archive_keeps_the_reply_in_the_inbox() {
        let scene = a_muted_thread(false).await;
        let (reply, thread) = scene
            .arrival("m2", &["m1"], &["ben@example.com"], &[], 2)
            .await;

        scene.files(&reply, thread, MailboxRole::Inbox).await;

        assert_eq!(
            scene.mailbox_of(&reply).await,
            scene.inbox,
            "nowhere to put it"
        );
        assert!(scene.handed_on(&reply));
        assert_eq!(scene.replies_since().await, Some(0), "and nothing counted");
    }

    #[tokio::test]
    async fn the_mute_check_is_one_statement_for_the_batch_and_none_for_sent_mail() {
        let scene = a_muted_thread(true).await;
        let (one, thread) = scene
            .arrival("m2", &["m1"], &["ben@example.com"], &[], 2)
            .await;
        let (two, _) = scene
            .arrival("m3", &["m1"], &["ben@example.com"], &[], 3)
            .await;
        let (stranger, other) = scene.arrival("x1", &[], &["ben@example.com"], &[], 4).await;
        let batch = [
            FiledMessage {
                message: &one,
                thread: Some(thread),
                role: MailboxRole::Inbox,
            },
            FiledMessage {
                message: &two,
                thread: Some(thread),
                role: MailboxRole::Inbox,
            },
            FiledMessage {
                message: &stranger,
                thread: Some(other),
                role: MailboxRole::Inbox,
            },
        ];

        let counts = counting::counted_async(|| async {
            scene
                .pass
                .file(&scene.connection, &batch)
                .await
                .expect("filed");
        })
        .await;

        // The mute lookup for the page (one statement, one row), the own
        // addresses and the account's Archive (once each, however many
        // replies), and the key `replies_since` is counted on (once for the
        // thread). The moves and the count are writes, which `counting`
        // does not see.
        assert_eq!(scene.replies_since().await, Some(2), "both counted");
        assert_eq!(counts.statements, 4, "{counts:?}");

        let sent = [FiledMessage {
            message: &stranger,
            thread: Some(other),
            role: MailboxRole::Sent,
        }];
        let none = counting::counted_async(|| async {
            scene
                .pass
                .file(&scene.connection, &sent)
                .await
                .expect("filed");
        })
        .await;
        assert_eq!((none.statements, none.rows), (0, 0), "{none:?}");
    }
}
