//! A new reply ends a snooze (spec 013 D3, FR-004, research R1).
//!
//! Gmail's rule, clarified with the maintainer: a conversation that is not
//! muted comes back when somebody writes in it.
//!
//! - **Archived.** Nothing to do here. The reply is filed in the inbox like
//!   any arrival, and the conversation is a row there again because it has a
//!   member in the inbox.
//! - **Snoozed.** Snooze is per message (`messages.snoozed_until`), so the
//!   reply arrives awake while its conversation's other messages sleep, and
//!   the row would hide until their time. This wraps the filing pass and,
//!   once it has filed, wakes those sleepers with the `Unsnooze` verb's own
//!   write ([`MessageRepository::unsnooze_threads`]).
//! - **Labels.** The reply is not given the conversation's labels: the row
//!   shows their union already, and nothing is written to the server.
//!
//! Only mail received wakes a snooze. What the account sent itself, or filed
//! in Drafts, Junk or Trash, does not: a person's own reply should not bring
//! back a conversation they put to sleep.
//!
//! Cost: nothing for an arrival that joined no conversation, otherwise one
//! write for the whole batch, however many arrived, and no read.

use std::sync::Arc;

use postio_model::{MailboxRole, ThreadId};
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_sync::{FiledMessage, FilingEffects, FilingPass, SyncError};

/// The filing pass Focus runs: `inner`, then the snoozes of every
/// conversation a received message joined are ended.
#[derive(Debug)]
pub(crate) struct ReplyWakes {
    inner: Arc<dyn FilingPass>,
}

impl ReplyWakes {
    /// Wraps `inner`.
    pub(crate) fn wrapping(inner: Arc<dyn FilingPass>) -> Arc<dyn FilingPass> {
        Arc::new(Self { inner })
    }
}

#[async_trait::async_trait]
impl FilingPass for ReplyWakes {
    async fn file(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        // A muted thread's replies are archived first and judged by nothing
        // else, and they wake no snooze (spec 013 FR-012).
        let filed = &super::mute::archive_muted(transaction, filed).await?[..];
        let effects = self.inner.file(transaction, filed).await?;
        let threads = waking(filed);
        if !threads.is_empty() {
            // Not an error to the filing: the arrivals are filed all the
            // same, and a snooze that stays is the lesser harm.
            match MessageRepository::new(transaction)
                .unsnooze_threads(&threads)
                .await
            {
                Ok(woken) if woken > 0 => {
                    tracing::debug!(woken, "a reply ended a snooze");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "a reply could not end a snooze: {error}");
                }
            }
        }
        Ok(effects)
    }
}

/// The conversations whose sleepers `filed` wakes, each once.
fn waking(filed: &[FiledMessage<'_>]) -> Vec<ThreadId> {
    let mut threads: Vec<ThreadId> = filed
        .iter()
        .filter(|message| received(message.role))
        // Asleep itself: it was in the store when the conversation was
        // snoozed (catch-up, a resync), so it is not news. Only a live
        // arrival is born awake.
        .filter(|message| message.message.snoozed_until.is_none())
        .filter_map(|message| message.thread)
        // A muted reply never gets here: `mute::archive_muted` took it out.
        .collect();
    threads.sort_unstable();
    threads.dedup();
    threads
}

/// Whether mail filed into a folder of this role was received, rather than
/// written by the account or put away.
fn received(role: MailboxRole) -> bool {
    !matches!(
        role,
        MailboxRole::Sent
            | MailboxRole::Drafts
            | MailboxRole::Outbox
            | MailboxRole::Junk
            | MailboxRole::Trash
    )
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use postio_model::{AccountId, Flag, MailboxId, Message, Thread};
    use postio_storage::repository::ThreadRepository;
    use postio_storage::test_support::{self, counting};

    use super::*;

    #[derive(Debug)]
    struct Nothing;

    #[async_trait::async_trait]
    impl FilingPass for Nothing {
        async fn file(
            &self,
            _transaction: &Connection,
            _filed: &[FiledMessage<'_>],
        ) -> Result<FilingEffects, SyncError> {
            Ok(FilingEffects::default())
        }
    }

    fn later() -> DateTime<Utc> {
        Utc::now() + std::time::Duration::from_secs(86_400)
    }

    struct Scene {
        _database: postio_storage::Store,
        connection: postio_storage::Checkout,
        inbox: MailboxId,
        sent: MailboxId,
        account: AccountId,
        thread: ThreadId,
        earlier: Message,
    }

    /// A conversation with one earlier message, asleep, and an inbox.
    async fn a_snoozed_conversation() -> Scene {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let sent = test_support::mailbox(&connection, &account, "Sent")
            .await
            .id;
        let mut thread = Thread::new(account.id);
        ThreadRepository::new(&connection)
            .create(&mut thread)
            .await
            .expect("a thread");
        let earlier = member(&connection, account.id, inbox, thread.id, 10).await;
        MessageRepository::new(&connection)
            .snooze(&[earlier.id], later())
            .await
            .expect("asleep");
        Scene {
            _database: database,
            connection,
            inbox,
            sent,
            account: account.id,
            thread: thread.id,
            earlier,
        }
    }

    async fn member(
        connection: &Connection,
        account: AccountId,
        mailbox: MailboxId,
        thread: ThreadId,
        seconds: i64,
    ) -> Message {
        let at = DateTime::<Utc>::from_timestamp(1_800_000_000 + seconds, 0).expect("a time");
        let mut message = Message::new(account, mailbox, at);
        message.subject = Some("Tide gate".to_owned());
        message.flags = [Flag::Seen].into_iter().collect();
        MessageRepository::new(connection)
            .create(&mut message)
            .await
            .expect("a message");
        ThreadRepository::new(connection)
            .add_message(thread, message.id)
            .await
            .expect("joined");
        message
    }

    async fn asleep(scene: &Scene) -> usize {
        ThreadRepository::new(&scene.connection)
            .snoozed_messages(scene.thread)
            .await
            .expect("a read")
            .len()
    }

    async fn arrives(scene: &Scene, mailbox: MailboxId, role: MailboxRole) -> Message {
        let reply = member(&scene.connection, scene.account, mailbox, scene.thread, 20).await;
        let filed = [FiledMessage {
            message: &reply,
            thread: Some(scene.thread),
            role,
        }];
        ReplyWakes::wrapping(Arc::new(Nothing))
            .file(&scene.connection, &filed)
            .await
            .expect("filed");
        reply
    }

    #[tokio::test]
    async fn a_reply_into_a_snoozed_conversation_ends_the_snooze() {
        let scene = a_snoozed_conversation().await;
        assert_eq!(asleep(&scene).await, 1);

        arrives(&scene, scene.inbox, MailboxRole::Inbox).await;

        assert_eq!(asleep(&scene).await, 0, "the conversation is awake");
        let first = MessageRepository::new(&scene.connection)
            .get(scene.earlier.id)
            .await
            .expect("a read")
            .expect("there");
        assert_eq!(first.snoozed_until, None);
    }

    #[tokio::test]
    async fn a_message_that_is_itself_asleep_was_there_when_the_snooze_was_set() {
        // Catch-up and resyncs re-file old mail. A snooze writes every member
        // of the conversation, so an arrival that is asleep is no news.
        let scene = a_snoozed_conversation().await;
        let messages = MessageRepository::new(&scene.connection);
        let old = member(
            &scene.connection,
            scene.account,
            scene.inbox,
            scene.thread,
            20,
        )
        .await;
        messages.snooze(&[old.id], later()).await.expect("asleep");
        let old = messages.get(old.id).await.expect("read").expect("there");
        assert!(old.snoozed_until.is_some());
        let filed = [FiledMessage {
            message: &old,
            thread: Some(scene.thread),
            role: MailboxRole::Inbox,
        }];

        ReplyWakes::wrapping(Arc::new(Nothing))
            .file(&scene.connection, &filed)
            .await
            .expect("filed");

        assert_eq!(asleep(&scene).await, 2, "nobody woke");
    }

    #[tokio::test]
    async fn a_reply_the_account_sent_itself_leaves_the_snooze_standing() {
        let scene = a_snoozed_conversation().await;
        arrives(&scene, scene.sent, MailboxRole::Sent).await;
        assert_eq!(asleep(&scene).await, 1, "your own reply is not news");
    }

    #[tokio::test]
    async fn a_reply_into_an_archived_conversation_is_in_the_inbox() {
        // Guard: green on arrival. The reply is filed in the inbox like any
        // arrival, so the conversation is back with no help from here.
        let scene = a_snoozed_conversation().await;
        let messages = MessageRepository::new(&scene.connection);
        messages.unsnooze(&[scene.earlier.id]).await.expect("awake");

        let reply = arrives(&scene, scene.inbox, MailboxRole::Inbox).await;

        let stored = messages.get(reply.id).await.expect("read").expect("there");
        assert_eq!(stored.mailbox_id, scene.inbox);
        assert_eq!(stored.snoozed_until, None);
        assert!(!stored.sync.deleted_locally);
    }

    #[tokio::test]
    async fn the_reply_is_not_given_the_conversations_labels() {
        // Guard: green on arrival. Labels are not touched by filing; the row
        // shows the union of its members'.
        let scene = a_snoozed_conversation().await;
        let reply = arrives(&scene, scene.inbox, MailboxRole::Inbox).await;
        let stored = MessageRepository::new(&scene.connection)
            .get(reply.id)
            .await
            .expect("read")
            .expect("there");
        assert!(stored.labels.is_empty(), "arrived with none, has none");
    }

    #[tokio::test]
    async fn waking_reads_nothing_and_an_unrelated_arrival_costs_nothing() {
        let scene = a_snoozed_conversation().await;
        let wakes = ReplyWakes::wrapping(Arc::new(Nothing));

        let unrelated = Message::new(scene.account, scene.inbox, Utc::now());
        let alone = [FiledMessage {
            message: &unrelated,
            thread: None,
            role: MailboxRole::Inbox,
        }];
        let none = counting::counted_async(|| async {
            wakes.file(&scene.connection, &alone).await.expect("filed");
        })
        .await;
        assert_eq!((none.statements, none.rows), (0, 0), "no thread, no work");

        let reply = member(
            &scene.connection,
            scene.account,
            scene.inbox,
            scene.thread,
            30,
        )
        .await;
        let joined = [FiledMessage {
            message: &reply,
            thread: Some(scene.thread),
            role: MailboxRole::Inbox,
        }];
        let one = counting::counted_async(|| async {
            wakes.file(&scene.connection, &joined).await.expect("filed");
        })
        .await;
        // The counter sees reads; the wake is one write, which it does not.
        // The one read is the mute check, for the batch (spec 013 FR-012),
        // and it finds no mute: no rows.
        assert_eq!(
            (one.statements, one.rows),
            (1, 0),
            "it reads only the mutes"
        );
    }
}
