//! The one place a [`Request`] becomes a client call.
//!
//! Async, and tied to no executor: the GTK app awaits it on glib's main loop
//! and the FFI on tokio. Both get the same mapping, which is the point — the
//! aim of a command, the order of a scope's reads, and the shape of each
//! reply are decided here once (research R1).

use postio_client::Client;
use postio_model::listing::{ListPage, MailStore, PageRequest};
use postio_model::mailbox::MailboxRole;
use postio_model::{FocusScope, ListScope, MailboxId};

use postio_core::state::{SharedState, ViewScope};
use postio_core::{Command, MessageTarget};

use crate::{Found, FoundRow, Opened, PageAnswer, PlacesRead, Reply, Request};

/// Answer `request` now, when it needs no await: a post, said before
/// anything that follows it is asked. `Err` hands the request back for
/// [`perform()`].
///
/// Order is the point. Mail that left a folder is said to the store before
/// the list re-reads (`Request::NoteRemoved`), so the store's own caches
/// have let go of it first; spawned on an executor, the saying could run
/// after the re-read it was meant to precede.
// The request comes back whole, unboxed, because every caller hands it
// straight to `perform`: boxing it would only be unboxed again.
#[allow(clippy::result_large_err)]
pub fn perform_now(client: &Client, request: Request) -> Result<Reply, Request> {
    match request {
        Request::NoteRemoved { mailbox, messages } => {
            client.note_removed(mailbox, messages);
            Ok(Reply::Noted)
        }
        // A copy handed to the system's Quick Look or save panel is done
        // with (FR-053). Only ever one of `file_copies`' own: a path from
        // anywhere else is left alone.
        Request::RemoveCopy(path) => {
            crate::results::remove_copy(&path);
            Ok(Reply::Noted)
        }
        other => Err(other),
    }
}

/// Ask the engine; return what the controller is to be told.
pub async fn perform(client: &Client, request: Request) -> Reply {
    // How a passage treats the first line is the request's to say (D7).
    let first_line = request.first_line().unwrap_or_default();
    match request {
        Request::FocusCounts => Reply::FocusCounts(
            client
                .focus_counts()
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::OpenScope { scope, splices } => {
            // Which folders are inboxes: what lets Focus's inbox ignore mail
            // moving anywhere else rather than re-read on every arrival.
            let mut folders = Vec::new();
            let mut enabled = Vec::new();
            if let Ok(accounts) = client.accounts().await {
                for account in accounts.iter().filter(|account| account.enabled) {
                    enabled.push(account.id);
                    if let Ok(mailboxes) = client.mailboxes(account.id).await {
                        folders.extend(
                            mailboxes
                                .iter()
                                .map(|mailbox| (mailbox.id, mailbox.role == MailboxRole::Inbox)),
                        );
                    }
                }
            }
            let total = client
                .list_count(scope)
                .await
                .map_err(|error| error.to_string());
            let surfaced = match splices {
                true => Some(client.surfaced().await.map_err(|error| error.to_string())),
                false => None,
            };
            Reply::Opened(Opened {
                scope,
                accounts: enabled,
                folders,
                total,
                surfaced,
            })
        }
        Request::Page {
            page,
            stamp,
            start,
            count,
            wanted,
        } => {
            let answer = match client.list_page(wanted).await {
                Ok(ListPage::Threads(answer)) => {
                    // The page's label pills, for every row at once: one round
                    // trip, one statement at the store (spec 007 T043).
                    let threads = postio_ui::focus_list::label_threads(&answer.rows);
                    let labels = client.thread_labels(threads).await.unwrap_or_else(|error| {
                        tracing::warn!(page, %error, "Focus could not read a page's labels");
                        Vec::new()
                    });
                    Ok(PageAnswer::Threads {
                        total: answer.total,
                        rows: answer.rows,
                        labels,
                    })
                }
                Ok(ListPage::Messages(answer)) => Ok(PageAnswer::Messages {
                    total: answer.total,
                    rows: answer.rows,
                }),
                Err(error) => Err(error.to_string()),
            };
            Reply::Page {
                page,
                stamp,
                start,
                count,
                answer,
            }
        }
        Request::Surfaced => {
            Reply::Surfaced(client.surfaced().await.map_err(|error| error.to_string()))
        }
        Request::NoteRemoved { mailbox, messages } => {
            client.note_removed(mailbox, messages);
            Reply::Noted
        }
        Request::Send {
            command,
            aims,
            everything,
        } => Reply::Sent(send(client, command, aims, everything).await),
        Request::Post(command) => Reply::Sent(
            client
                .send(command)
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::Places => Reply::Places(places(client).await),
        Request::Search {
            query,
            order,
            stamp,
        } => Reply::Search {
            stamp,
            answer: search(client, query, order).await,
        },
        Request::Conversations {
            query,
            order,
            limit,
            stamp,
        } => Reply::Conversations {
            stamp,
            answer: client
                .conversations(postio_model::AccountScope::Unified, query, order, 0, limit)
                .await
                .map_err(|error| error.to_string())
                .and_then(|found| found.ok_or_else(|| "no search index here".to_owned()))
                .map(Box::new),
        },
        Request::Passages { query, hits, stamp } => Reply::Passages {
            stamp,
            answer: client
                .passages(query, hits, first_line)
                .await
                .map_err(|error| error.to_string()),
        },
        Request::ResultsPage {
            query,
            order,
            offset,
            limit,
            stamp,
        } => Reply::ResultsPage {
            stamp,
            order,
            offset,
            answer: client
                .conversations(
                    postio_model::AccountScope::Unified,
                    query,
                    order,
                    offset,
                    limit,
                )
                .await
                .map_err(|error| error.to_string())
                .and_then(|found| found.ok_or_else(|| "no search index here".to_owned()))
                .map(Box::new),
        },
        Request::ResultsPassages { query, hits, stamp } => Reply::ResultsPassages {
            stamp,
            answer: client
                .passages(query, hits, first_line)
                .await
                .map_err(|error| error.to_string()),
        },
        Request::Files { query, stamp } => Reply::Files {
            stamp,
            answer: client
                .files(
                    postio_model::AccountScope::Unified,
                    query,
                    0,
                    crate::results::FILES_READ,
                )
                .await
                .map_err(|error| error.to_string()),
        },
        Request::People { query, stamp } => Reply::People {
            stamp,
            answer: client
                .people(
                    postio_model::AccountScope::Unified,
                    query,
                    0,
                    crate::results::PEOPLE_READ,
                )
                .await
                .map_err(|error| error.to_string()),
        },
        Request::AttachmentCopy {
            attachment,
            purpose,
            stamp,
        } => Reply::AttachmentCopy {
            stamp,
            purpose,
            answer: client
                .attachment_copy(attachment, crate::results::file_copies())
                .await
                .map_err(|error| error.to_string()),
        },
        Request::RemoveCopy(path) => {
            crate::results::remove_copy(&path);
            Reply::Noted
        }
        Request::QuickLookMatches { query, key, stamp } => Reply::QuickLookMatches {
            stamp,
            answer: client
                .conversation_matches(query, key)
                .await
                .map_err(|error| error.to_string()),
        },
        Request::Relaxations {
            query,
            today,
            stamp,
        } => Reply::Relaxations {
            stamp,
            answer: client
                .relaxations(postio_model::AccountScope::Unified, query, today)
                .await
                .map_err(|error| error.to_string()),
        },
        // The facets alone: no hit is read.
        Request::Facets { query, stamp } => Reply::Facets {
            stamp,
            answer: client
                .conversations(
                    postio_model::AccountScope::Unified,
                    query,
                    postio_search::results::ConversationOrder::Newest,
                    0,
                    0,
                )
                .await
                .map_err(|error| error.to_string())
                .and_then(|found| found.ok_or_else(|| "no search index here".to_owned()))
                .map(Box::new),
        },
        Request::Suggest {
            prefix,
            field,
            stamp,
        } => Reply::Suggest {
            stamp,
            answer: client
                .suggest(postio_model::AccountScope::Unified, prefix, field)
                .await
                .map(Box::new)
                .map_err(|error| error.to_string()),
        },
        // Two, newest first: the "Latest from" preview under the people.
        Request::LatestFrom { address, stamp } => {
            let query = postio_search::parse(
                &postio_search::query::spell(&postio_search::query::Clause {
                    negated: false,
                    filter: postio_search::query::Filter::From(address.clone()),
                }),
                postio_ui::clock::now().date_naive(),
            );
            Reply::LatestFrom {
                stamp,
                address,
                answer: client
                    .conversations(
                        postio_model::AccountScope::Unified,
                        query,
                        postio_search::results::ConversationOrder::Newest,
                        0,
                        crate::dropdown::LATEST,
                    )
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|found| found.ok_or_else(|| "no search index here".to_owned()))
                    .map(Box::new),
            }
        }
        Request::RecentSearches => Reply::RecentSearches(
            client
                .recent_searches()
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::RememberSearch { query, hits } => {
            if let Err(error) = client.remember_search(query, hits).await {
                tracing::warn!(%error, "Focus could not keep a recent search");
            }
            Reply::Noted
        }
        // Answered with what is left, so the panel needs no second read.
        Request::ForgetSearch { query } => {
            if let Err(error) = client.forget_search(query).await {
                tracing::warn!(%error, "Focus could not forget a recent search");
            }
            Reply::RecentSearches(
                client
                    .recent_searches()
                    .await
                    .map_err(|error| error.to_string()),
            )
        }
        Request::MarkSeen { key } => {
            if let Err(error) = client.mark_seen(key).await {
                tracing::warn!(%error, "Focus could not mark a saved search seen");
            }
            Reply::Noted
        }
        Request::SavedCounts { searches, today } => Reply::SavedCounts(
            client
                .saved_counts(postio_model::AccountScope::Unified, today, searches)
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::Folder { mailbox, stamp } => {
            let scope = ListScope::Mailbox(mailbox);
            let count = client
                .list_count(scope)
                .await
                .map_err(|error| error.to_string());
            let rows = client
                .list_page(PageRequest {
                    scope,
                    offset: 0,
                    limit: postio_ui::command_bar::FOLDER_ROWS,
                })
                .await
                .map(folder_rows)
                .map_err(|error| error.to_string());
            Reply::Folder { stamp, count, rows }
        }
        Request::RoleFolder(role) => Reply::RoleFolder(role_folder(client, role).await),
        Request::Labels {
            message,
            account,
            threads,
            stamp,
        } => Reply::Labels {
            stamp,
            answer: labels(client, message, account, threads).await,
        },
        Request::CreateLabel {
            account,
            name,
            stamp,
        } => Reply::LabelCreated {
            stamp,
            answer: match client.create_label(account, name).await {
                Ok(Some(label)) => Ok(label),
                Ok(None) => Err("the label was not made".to_owned()),
                Err(error) => Err(error.to_string()),
            },
        },
        Request::Folders { stamp } => Reply::Folders {
            stamp,
            answer: folders(client).await,
        },
        Request::NoteMove(mailbox) => {
            if let Err(error) = client.note_move(mailbox).await {
                tracing::warn!(%error, "Focus could not keep a recent move");
            }
            Reply::Noted
        }
        Request::Accounts => Reply::Accounts(accounts(client).await),
        Request::FilteredTabs => Reply::FilteredTabs(
            client
                .filtered_tabs()
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::Filtered {
            reason,
            offset,
            stamp,
        } => Reply::Filtered {
            stamp,
            offset,
            answer: client
                .filtered(reason, offset, postio_ui::filtered::PAGE)
                .await
                .map_err(|error| error.to_string()),
        },
        Request::SweepPreview => Reply::SweepPreview(
            client
                .sweep_preview()
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::DigestRead {
            delivery,
            summary,
            stamp,
        } => {
            let rows = client
                .delivery_messages(delivery)
                .await
                .map_err(|error| error.to_string());
            // C6: a summary is read only when the person brought a model;
            // one that cannot be read is no summary, and the list opens.
            let summary = match summary {
                true => client.digest_summary(delivery).await.ok().flatten(),
                false => None,
            };
            Reply::DigestRead {
                stamp,
                rows,
                summary,
            }
        }
        Request::Unsubscribe(message) => Reply::Unsubscribed(
            client
                .unsubscribe(message)
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::DigestPreview {
            queries,
            since,
            stamp,
        } => Reply::DigestPreview {
            stamp,
            answer: client
                .digest_preview(queries, since)
                .await
                .map_err(|error| error.to_string()),
        },
        Request::DigestLikeThis { message, stamp } => Reply::LikeThis {
            stamp,
            answer: client
                .digest_like_this(message)
                .await
                .map(|rule| rule.map(|rule| rule.queries))
                .map_err(|error| error.to_string()),
        },
        Request::SaveDigestRule {
            replacing,
            draft,
            stamp,
        } => Reply::RuleSaved {
            stamp,
            answer: client
                .save_digest_rule(replacing, draft)
                .await
                .map_err(|error| error.to_string()),
        },
        Request::Vault { subject, stamp } => Reply::Vault {
            stamp,
            answer: client
                .vault(&subject)
                .await
                .map_err(|error| error.to_string()),
        },
        Request::CaptureTask {
            project,
            task,
            stamp,
        } => Reply::Captured {
            stamp,
            answer: client
                .capture_task(project, task)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string()),
        },
        Request::CaptureNote { note, entry, stamp } => Reply::Captured {
            stamp,
            answer: client
                .capture_note(note, entry)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string()),
        },
        Request::FindMessage(message) => Reply::FoundMessage {
            message,
            row: client
                .message_rows(vec![message])
                .await
                .ok()
                .and_then(|rows| rows.into_iter().find(|row| row.id == message)),
        },
        // A message that is no draft, or one the store cannot say, is
        // answered alike: there is nothing to settle.
        Request::DraftBehind { message, command } => Reply::DraftBehind {
            message,
            command,
            draft: client
                .draft_behind(message)
                .await
                .ok()
                .flatten()
                .map(|draft| draft.id),
        },
    }
}

/// Who every enabled account is, as a banner names it, and the newest of
/// their folders' last completed syncs: when mail last arrived before this
/// run. GTK's window read the same when its accounts landed.
async fn accounts(client: &Client) -> Result<crate::AccountsRead, String> {
    let accounts = client.accounts().await.map_err(|error| error.to_string())?;
    let mut read = crate::AccountsRead::default();
    for account in accounts.iter().filter(|account| account.enabled) {
        read.facts.push(postio_ui::focus_state::AccountFacts {
            id: account.id,
            server: account.incoming.host.clone(),
            address: account.address.address.clone(),
            name: if account.display_name.is_empty() {
                account.address.address.clone()
            } else {
                account.display_name.clone()
            },
        });
        if let Ok(mailboxes) = client.mailboxes(account.id).await {
            read.last_synced = read.last_synced.max(
                mailboxes
                    .iter()
                    .filter_map(|mailbox| mailbox.last_synced_at)
                    .max(),
            );
        }
    }
    Ok(read)
}

/// Every place there is to go, in one round: each enabled account's
/// mailboxes and folders with their conversations counted, its Outbox while
/// anything waits in it, its labels with how many carry each, and its
/// correspondents; then Snoozed and Flagged, which span every account.
/// GTK's popover and bar each read these themselves, one call at a time.
async fn places(client: &Client) -> Result<PlacesRead, String> {
    use postio_ui::places as rules;
    let accounts = client.accounts().await.map_err(|error| error.to_string())?;
    let enabled: Vec<_> = accounts.iter().filter(|account| account.enabled).collect();
    let mut read = PlacesRead::default();
    let mut owners = Vec::new();
    for account in &enabled {
        for mailbox in client.mailboxes(account.id).await.unwrap_or_default() {
            owners.push((mailbox.id, account.address.address.clone()));
            read.folders.push((mailbox.id, rules::place_name(&mailbox)));
            read.places.push(rules::mailbox_place(&mailbox));
            let mut entry = rules::mailbox_entry(&mailbox);
            // Conversations, as the strip counts them: the inbox's is
            // Focus's, which is what the strip reads.
            let scope = if mailbox.role == MailboxRole::Inbox {
                ListScope::Focus(FocusScope::Inbox)
            } else {
                ListScope::Mailbox(mailbox.id)
            };
            if let Ok(conversations) = client.list_count(scope).await {
                entry.count = Some(conversations.to_string());
            }
            read.entries.push(entry);
        }
        // The Outbox, a view over Drafts, has no mailbox row of its own.
        let waiting = client
            .list_count(ListScope::Outbox(account.id))
            .await
            .unwrap_or(0);
        if waiting > 0 {
            read.entries.push(rules::outbox_entry(account.id, waiting));
        }
        let counted = client.label_counts(account.id).await.unwrap_or_default();
        for label in client.labels(account.id).await.unwrap_or_default() {
            // A label nothing carries is left out of the counts: it holds none.
            let held = counted
                .iter()
                .find(|(id, _)| *id == label.id)
                .map_or(0, |(_, held)| *held);
            read.entries.push(rules::label_entry(&label, Some(held)));
            read.places.push(rules::label_place(&label));
        }
        read.contacts
            .extend(client.correspondents(account.id).await.unwrap_or_default());
    }
    for (role, scope) in [
        (MailboxRole::Snoozed, FocusScope::Snoozed),
        (MailboxRole::Flagged, FocusScope::Flagged),
    ] {
        let held = client.list_count(ListScope::Focus(scope)).await.ok();
        let view = rules::view_entry(role, held);
        // A mailbox the server calls Flagged is this view's row.
        read.entries.retain(|entry| entry.go != view.go);
        read.entries.push(view);
    }
    // Which account a hit is from is said only where there are several.
    if enabled.len() > 1 {
        read.owners = owners;
    }
    Ok(read)
}

/// Search this machine's index for the bar: one row per conversation, and
/// where a digest holds any of them.
async fn search(
    client: &Client,
    query: postio_search::ParsedQuery,
    order: postio_search::ResultOrder,
) -> Result<Found, String> {
    let results = client
        .search_hits(
            postio_model::AccountScope::Unified,
            query,
            postio_search::facets::Scope::AllMail,
            order,
            0,
        )
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "nothing to search for".to_owned())?;
    let hits = postio_ui::command_bar::conversations(results.hits);
    let held = if hits.is_empty() {
        Vec::new()
    } else {
        let ids = hits.iter().map(|hit| hit.message_id).collect();
        client.held(ids).await.unwrap_or_default()
    };
    Ok(Found {
        hits,
        instead: results.instead,
        held,
    })
}

/// What the label picker lists, in one round: the labels of `message`'s
/// account -- a message can carry only its own account's (T170) -- or of
/// `account`, or of the first enabled one; how many conversations carry
/// each; and which of them `threads` carry.
async fn labels(
    client: &Client,
    message: Option<postio_model::MessageId>,
    account: Option<postio_model::AccountId>,
    threads: Vec<postio_model::ThreadId>,
) -> Result<crate::LabelsRead, String> {
    let owner = match message {
        Some(message) => client.account_of(message).await.ok().flatten(),
        None => None,
    };
    let account = match owner.or(account) {
        Some(account) => account,
        None => client
            .accounts()
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|account| account.enabled)
            .map(|account| account.id)
            .ok_or_else(|| "there is no account to label in".to_owned())?,
    };
    let labels = client
        .labels(account)
        .await
        .map_err(|error| error.to_string())?;
    let counts = client.label_counts(account).await.unwrap_or_default();
    let carried = if threads.is_empty() {
        Vec::new()
    } else {
        client.thread_labels(threads).await.unwrap_or_default()
    };
    Ok(crate::LabelsRead {
        account,
        labels,
        counts,
        carried,
    })
}

/// What the move picker lists, in one round: every enabled account's
/// folders, and the last destinations.
async fn folders(client: &Client) -> Result<crate::FoldersRead, String> {
    let accounts = client.accounts().await.map_err(|error| error.to_string())?;
    let mut folders = Vec::new();
    for account in accounts.iter().filter(|account| account.enabled) {
        folders.extend(client.mailboxes(account.id).await.unwrap_or_default());
    }
    let recent = client.move_recent().await.unwrap_or_default();
    Ok(crate::FoldersRead { folders, recent })
}

/// A folder's first page, as the bar lists it.
fn folder_rows(page: ListPage) -> Vec<FoundRow> {
    use postio_ui::command_bar::said_of;
    match page {
        ListPage::Threads(page) => page
            .rows
            .into_iter()
            .map(|thread| FoundRow {
                message: thread.representative.id,
                from: thread.representative.from.as_ref().map(said_of),
                subject: thread.subject.clone().unwrap_or_default(),
                preview: thread.representative.preview.clone(),
                at: thread.last_at,
            })
            .collect(),
        ListPage::Messages(page) => page
            .rows
            .into_iter()
            .map(|message| FoundRow {
                message: message.id,
                from: message.from.as_ref().map(said_of),
                subject: message.subject.clone().unwrap_or_default(),
                preview: message.preview.clone(),
                at: message.received_at,
            })
            .collect(),
    }
}

/// The folder of `role` in the account Focus writes from: the first enabled
/// account with one. A role, not a name: what a provider calls its sent
/// mail does not matter.
async fn role_folder(client: &Client, role: MailboxRole) -> Option<(MailboxId, String)> {
    let accounts = client.accounts().await.ok()?;
    for account in accounts.iter().filter(|account| account.enabled) {
        let Ok(folders) = client.mailboxes(account.id).await else {
            continue;
        };
        if let Some(folder) = folders.iter().find(|folder| folder.role == role) {
            return Some((folder.id, postio_ui::places::place_name(folder)));
        }
    }
    None
}

/// A verb, to each aim in turn; or, for a whole-view selection, once at the
/// selection the host is told about with it -- the one place a frontend's
/// "select all" becomes the host's (GTK and the FFI each had a copy).
async fn send(
    client: &Client,
    command: Command,
    aims: Vec<MessageTarget>,
    everything: Option<crate::Everything>,
) -> Result<(), String> {
    if let Some(crate::Everything {
        query: Some(query),
        except,
        ..
    }) = everything
    {
        return client
            .send_matching(command, postio_model::AccountScope::Unified, query, except)
            .await
            .map_err(|error| error.to_string());
    }
    if let Some(everything) = everything {
        let state = SharedState::default();
        let (sink, _) = postio_core::bridge::event_channel();
        state.update(&sink, |app| {
            let mut events = app.open_view(ViewScope::Focus {
                accounts: everything.accounts.clone(),
            });
            events.extend(app.select_all());
            for message in &everything.except {
                events.extend(app.toggle_selection(*message));
            }
            events
        });
        return client
            .clone()
            .with_state(state)
            .send(command.with_target(MessageTarget::Selection))
            .await
            .map_err(|error| error.to_string());
    }
    for aim in aims {
        client
            .send(command.clone().with_target(aim))
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    use postio_client::Transport;
    use postio_client::api::Call;
    use postio_client::protocol::{FocusCounts, Req, Resp};
    use postio_core::EventEnvelope;

    use super::*;

    /// Answers from a script and records what it was asked.
    struct Scripted {
        asked: Mutex<Vec<Req>>,
        answers: Mutex<Vec<Resp>>,
        events: async_channel::Receiver<EventEnvelope>,
    }

    impl Transport for Scripted {
        fn call(&self, request: Req) -> Call<'static> {
            self.asked.lock().unwrap().push(request);
            let answer = self.answers.lock().unwrap().remove(0);
            Box::pin(async move { Ok(answer) })
        }
        fn post(&self, request: Req) {
            self.asked.lock().unwrap().push(request);
        }
        fn events(&self) -> async_channel::Receiver<EventEnvelope> {
            self.events.clone()
        }
    }

    fn client(answers: Vec<Resp>) -> (Client, Arc<Scripted>) {
        let (_tx, events) = async_channel::unbounded();
        let transport = Arc::new(Scripted {
            asked: Mutex::new(Vec::new()),
            answers: Mutex::new(answers),
            events,
        });
        (Client::new(transport.clone()), transport)
    }

    /// The scripted transport answers at once, so one poll finishes it.
    fn now<F: Future>(future: F) -> F::Output {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("a scripted answer is ready on the first poll"),
        }
    }

    #[test]
    fn counts_are_one_focus_counts_request() {
        let counts = FocusCounts {
            conversations: 312,
            unread: 41,
            has_action: 7,
            filtered_today: 186,
        };
        let (client, transport) = client(vec![Resp::FocusCounts(counts)]);
        assert_eq!(
            now(perform(&client, Request::FocusCounts)),
            Reply::FocusCounts(Ok(counts)),
        );
        assert_eq!(*transport.asked.lock().unwrap(), vec![Req::FocusCounts]);
    }

    #[test]
    fn a_digest_read_asks_for_its_summary_only_when_told_to() {
        let delivery = postio_model::DeliveryId::new(40);
        let (listing, transport) = client(vec![Resp::Rows(Vec::new())]);
        assert_eq!(
            now(perform(
                &listing,
                Request::DigestRead {
                    delivery,
                    summary: false,
                    stamp: 3,
                }
            )),
            Reply::DigestRead {
                stamp: 3,
                rows: Ok(Vec::new()),
                summary: None,
            },
        );
        assert_eq!(
            *transport.asked.lock().unwrap(),
            vec![Req::DeliveryMessages(delivery)],
            "C6: no model, no summary read"
        );
        let (summarised, transport) =
            client(vec![Resp::Rows(Vec::new()), Resp::DigestSummary(None)]);
        let _ = now(perform(
            &summarised,
            Request::DigestRead {
                delivery,
                summary: true,
                stamp: 4,
            },
        ));
        assert_eq!(
            *transport.asked.lock().unwrap(),
            vec![
                Req::DeliveryMessages(delivery),
                Req::DigestSummary(delivery)
            ],
        );
    }

    #[test]
    fn a_link_is_looked_up_by_its_row() {
        let (client, transport) = client(vec![Resp::Rows(Vec::new())]);
        let message = postio_model::MessageId::new(42);
        assert_eq!(
            now(perform(&client, Request::FindMessage(message))),
            Reply::FoundMessage { message, row: None },
        );
        assert_eq!(
            *transport.asked.lock().unwrap(),
            vec![Req::Rows(vec![message])]
        );
    }
}
