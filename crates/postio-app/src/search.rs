//! Search, joined to the store.
//!
//! `postio-gtk` built the whole search surface and cannot run a search: it may
//! not link SQLite, so the box paces the query and then hands it to whoever
//! owns the store. That is this crate. Everything here is the other half of a
//! seam `postio-gtk` deliberately left open — [`Live::connect_run`],
//! [`View::set_facets`], [`View::set_focused`], [`Finder::set_contacts`] — and
//! until it existed, typing in the box did nothing at all (`postio-1ag`).
//!
//! # Two round trips, not one
//!
//! A run answers the readout first and the columns second. They are separate
//! reads because they cost differently and are wanted differently: the hit
//! count is what the user is watching the field for and has to land as soon as
//! it can, while the facets are three scope counts plus a refinement pass over
//! the same result set, and nobody is waiting on them with their fingers
//! still moving. Folding them into one read would make the number people watch
//! wait for the numbers they do not.
//!
//! Neither runs per keystroke: [`Live`] debounces, and a run only reaches here
//! when the typing has stopped, `Enter` was pressed, or the scope changed.
//!
//! # Crossing the two loops
//!
//! The same arrangement as [`crate::feed`], for the same reason: `rusqlite` is
//! blocking and the GTK main loop must never be inside a query. The read goes
//! to the runtime's blocking pool and the answer comes back over an
//! `async_channel` that both loops can wait on.
//!
//! # Superseded answers are dropped whole
//!
//! [`Live::deliver`] returns `false` when the query it answers has already
//! been replaced. Everything derived from those results goes with it — the
//! facets, the preview, the event — because a surface filled from a query
//! nobody is asking any more is worse than one that has not caught up yet.
//!
//! [`Live`]: postio_gtk::search::Live
//! [`Live::connect_run`]: postio_gtk::search::Live::connect_run
//! [`Live::deliver`]: postio_gtk::search::Live::deliver
//! [`Finder::set_contacts`]: postio_gtk::finder::Finder::set_contacts

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::WidgetExt;
use postio_core::bridge::CommandSender;
use postio_core::{Command, Event};
use postio_gtk::feed::Feeds;
use postio_gtk::finder::Finder;
use postio_gtk::search::{Outcome, View};
use postio_gtk::window::Window;
use postio_index::SearchRequest;
use postio_model::AccountScope;
use postio_model::ids::AccountId;
use postio_search::facets::{Facets, Scope};
use postio_search::{ParsedQuery, SearchResults};
// `run`, `snippet_hits`, `HIT_LIMIT` and `SNIPPET_HITS` moved to
// `postio_session::search` in #660, so the macOS frontend runs the same search
// rather than a second one with its own hit limit and its own excerpt rule.
// `facets` below still needs the limit, which is why it is imported and not
// merely called through.
use postio_session::search::{HIT_LIMIT, execute as run};
use postio_storage::repository::ContactRepository;
use postio_storage::{Database, PooledConnection};

use crate::Wiring;

/// Wire the search surfaces to the store.
///
/// Called once, at window build, from the same place the panes are fed.
/// `None` when the store holds no account: there is nothing to search, and the
/// box says so by finding nothing rather than by being wired to an account
/// that does not exist.
/// `feeds` is what makes the results reach the message list. It is not
/// optional in the running application — `feed_the_window` builds both — and
/// is taken by reference here rather than found, because which `Feeds` a
/// window has is the composition root's business, the same as the source.
pub fn install(window: &Window, wiring: &Wiring, feeds: &Feeds) -> Option<View> {
    let account = crate::first_account(&wiring.database)?;
    let finder = window.finder();
    let view = View::attach(&window.shell(), &finder);

    // The hits the surfaces are drawn from, shared between the run that
    // produces them and the cursor that walks them.
    let held: Held = Rc::new(std::cell::RefCell::new(None));

    // Which order the result set is in (#499). Owned here, beside the scope,
    // because the same run reads both: the executor is asked for ranked or
    // date order per request, and the list header reports whichever the
    // rows are actually in.
    let order: Order = Rc::new(std::cell::Cell::new(postio_search::ResultOrder::default()));

    install_leave_to_list(window, &finder);
    install_preview(&view, wiring);
    install_run(
        &view,
        &finder,
        account.id,
        wiring,
        held.clone(),
        order.clone(),
    );
    install_results(window, feeds, &view, held, wiring, order.clone());
    install_order_toggle(window, &finder, feeds, order);
    load_contacts(&finder, account.id, wiring);

    Some(view)
}

/// Once a search runs, or `Tab` has nothing to refine, the keyboard moves to
/// the message list rather than staying in the field or falling through to
/// an unpredictable GTK focus-chain destination (#693).
///
/// The `Tab` handler is registered after [`View::attach`]'s own
/// `connect_tab` (canvas 2b's `Tab refine`), which is what lets this run
/// only when that one did not: `Finder::press_tab`'s handlers try in
/// registration order and stop at the first one that claims the keyboard, so
/// a refine chip still wins when there is one to move to.
fn install_leave_to_list(window: &Window, finder: &Finder) {
    finder.connect_search({
        let window = window.clone();
        move |_parsed| {
            window.list().grab_focus();
        }
    });
    finder.connect_tab({
        let window = window.clone();
        move || {
            window.list().grab_focus();
            true
        }
    });
}

/// The order the current result set is in. See [`install`].
type Order = Rc<std::cell::Cell<postio_search::ResultOrder>>;

/// Answer [`CommandId::ToggleResultOrder`] — `o` over results, or a click on
/// the list header's sort control.
///
/// Toggles, relabels, and asks the same query again in the new order. Only
/// while the list is showing results: over a mailbox there is no other order
/// to offer, and the control is inert.
fn install_order_toggle(window: &Window, finder: &Finder, feeds: &Feeds, order: Order) {
    window.connect_command({
        let window = window.clone();
        let finder = finder.clone();
        let feeds = feeds.clone();
        move |id| {
            if id != postio_core::CommandId::ToggleResultOrder || !feeds.messages.showing_results()
            {
                return;
            }
            let next = order.get().toggled();
            order.set(next);
            window.list().set_result_order(Some(next));
            if let Some(live) = finder.live() {
                live.rerun();
            }
        }
    });
}

/// Read the store on the runtime and answer over a channel.
///
/// `work` runs on the blocking pool with a connection of its own. `None` from
/// it — or a connection that could not be checked out — reaches the caller as
/// `None`, which every caller here treats as "draw nothing", because a search
/// that could not run has no answer and must not invent one.
pub(crate) fn ask<T, F>(database: &Database, runtime: &tokio::runtime::Handle, work: F) -> Answer<T>
where
    T: Send + 'static,
    F: FnOnce(&PooledConnection) -> Option<T> + Send + 'static,
{
    let (sender, receiver) = async_channel::bounded(1);
    let database = database.clone();
    runtime.spawn_blocking(move || {
        let answer = database
            .connection()
            .map_err(|error| tracing::warn!(%error, "no connection to read the index with"))
            .ok()
            .and_then(|connection| work(&connection));
        let _ = sender.send_blocking(answer);
    });
    receiver
}

/// What [`ask`] hands back: one answer, or none.
type Answer<T> = async_channel::Receiver<Option<T>>;

/// Run a search when the box says a query is due.
fn install_run(
    view: &View,
    finder: &Finder,
    account: AccountId,
    wiring: &Wiring,
    held: Held,
    order: Order,
) {
    let Some(live) = finder.live() else {
        // The readout is built by `Finder::attach`, which the window does
        // before this runs. If it is ever missing, search silently does
        // nothing — which is the bug this module exists to fix, so it is worth
        // a line rather than a `return` nobody sees.
        tracing::error!("the search box has no readout; nothing will answer a query");
        return;
    };

    let database = wiring.database.clone();
    let runtime = wiring.runtime.clone();
    let events = wiring.events.clone();
    let view = view.clone();

    live.connect_run({
        let live = live.clone();
        move |parsed, sequence| {
            // Owned, because the read happens on another thread and the box
            // is free to keep typing while it does.
            let query = parsed.clone();
            let scope = view.scope();
            // Read on this side of the thread hop: the cell lives with the
            // GTK loop, and the value — `Copy` — travels with the work.
            let order = order.get();
            let hits = ask(&database, &runtime, {
                let query = query.clone();
                move |connection| run(connection, account, &query, scope, order)
            });

            glib::spawn_future_local({
                let live = live.clone();
                let view = view.clone();
                let database = database.clone();
                let runtime = runtime.clone();
                let events = events.clone();
                let held = held.clone();
                async move {
                    let Ok(Some(results)) = hits.recv().await else {
                        // The store could not be read, so there is no answer
                        // coming. Saying so is what lets the box send out
                        // whatever query queued up behind this run — the
                        // single-flight rule holds it until the outstanding
                        // run resolves, one way or the other.
                        live.settled(sequence);
                        return;
                    };
                    // Counts, a scope and a duration: never the query text or
                    // what it matched, which are the user's mail. The same
                    // line that tells a search which ran and found nothing
                    // from one that never ran at all — the distinction that
                    // took `postio-x4e` and `postio-qhz.7` far too long.
                    tracing::debug!(
                        ?scope,
                        hits = results.hits.len(),
                        total = results.total_hits,
                        capped = results.total_hits_capped,
                        elapsed_ms = results.elapsed.as_millis() as u64,
                        "search answered"
                    );
                    // The readout first: it is what the field is showing and
                    // what the user is waiting for.
                    if !live.deliver(sequence, Outcome::of(&results)) {
                        // Superseded. Everything downstream of these results
                        // is about a question nobody is asking.
                        return;
                    }
                    // Held before it is announced: the event puts the hits in
                    // the list, which moves the cursor, which looks them up
                    // here. Announcing first would race the cursor against the
                    // results it is a cursor into.
                    focus(&view, &results, &database, &runtime);
                    held.replace(Some(results));
                    // Scoped, so the borrow is gone before `facets` runs:
                    // nothing downstream needs `held` today, and a borrow
                    // left open across two calls is a `borrow_mut` panic
                    // waiting for whoever edits this next.
                    if let Some(results) = held.borrow().as_ref() {
                        announce(&events, &query, results);
                    }
                    facets(
                        &view, &live, sequence, account, &query, scope, &database, &runtime,
                    );
                }
            });
        }
    });
}

/// The second round trip: what the columns say about this result set.
///
/// Checked against `sequence` again when it lands, because it is a whole extra
/// read behind an answer that was current when it started and may not be by
/// the time it finishes.
#[allow(clippy::too_many_arguments)]
fn facets(
    view: &View,
    live: &postio_gtk::search::Live,
    sequence: u64,
    account: AccountId,
    query: &ParsedQuery,
    scope: Scope,
    database: &Database,
    runtime: &tokio::runtime::Handle,
) {
    let answer = ask(database, runtime, {
        let query = query.clone();
        move |connection| {
            postio_index::executor::facets(
                connection,
                &SearchRequest {
                    // See `run` for why this is not `Unified` yet.
                    account: AccountScope::Account(account),
                    query: &query,
                    scope,
                    limit: HIT_LIMIT,
                    order: postio_search::ResultOrder::Relevance,
                },
            )
            .map_err(|error| tracing::warn!(%error, "the facet counts did not run"))
            .ok()
        }
    });
    glib::spawn_future_local({
        let view = view.clone();
        let live = live.clone();
        async move {
            let Ok(Some(facets)) = answer.recv().await else {
                return;
            };
            if live.outstanding() != sequence {
                return;
            }
            let total = total_in_scope(&facets, scope);
            view.set_facets(&facets, total);
        }
    });
}

/// How many hits the scope being looked at holds, out of the facet counts.
///
/// The panel draws this as the denominator its refinement chips narrow, so it
/// has to be the count for the scope on screen rather than the account-wide
/// one — `Facets` carries every scope precisely so the column can say what
/// *switching* would find.
fn total_in_scope(facets: &Facets, scope: Scope) -> u64 {
    facets
        .scopes
        .iter()
        .find(|count| count.scope == scope)
        .map(|count| count.hits)
        .unwrap_or_default()
}

/// The hits the surfaces are currently drawn from.
///
/// Held because the cursor moving through the list arrives as a `MessageId`
/// and [`View::set_focused`] wants the whole `SearchHit` — the snippet, the
/// sender and the mailbox all come from the index, not from the row. At most
/// `HIT_LIMIT` of them, so the lookup is a scan and does not need to be
/// anything cleverer.
type Held = Rc<std::cell::RefCell<Option<SearchResults>>>;

/// Preview the best match, and fetch its body.
///
/// The best match is what the canvas draws for a query just typed, and it is
/// where the list's cursor lands — so this paints the first frame and
/// `follow_cursor` takes over from the next keystroke on. Both go through
/// [`preview`], so there is one path to the pane rather than two that can
/// disagree.
fn focus(
    view: &View,
    results: &SearchResults,
    database: &Database,
    runtime: &tokio::runtime::Handle,
) {
    view.set_focused(results.hits.first());
    let Some(hit) = results.hits.first() else {
        return;
    };
    preview(view, hit, database, runtime);
}

/// Draw `hit`'s body into the preview.
fn preview(
    view: &View,
    hit: &postio_search::SearchHit,
    database: &Database,
    runtime: &tokio::runtime::Handle,
) {
    // The snippet is already on screen — highlighted, from the index — so this
    // is the body arriving under it rather than the pane waiting on a read to
    // show anything at all.
    let message = hit.message_id;
    let sender = hit.from.as_ref().map(|from| from.address.clone());
    let answer = ask(database, runtime, move |connection| {
        Some(crate::compose::load_body(connection, message))
    });
    glib::spawn_future_local({
        let view = view.clone();
        async move {
            let Ok(Some(body)) = answer.recv().await else {
                return;
            };
            let preview = view.preview();
            // The focus may have moved on while the body was read. Painting
            // a body into a preview showing a different message would be
            // worse than leaving the snippet alone.
            if preview.focused() != Some(message) {
                return;
            }
            preview.set_body(message, &body, sender.as_deref());
        }
    });
}

/// Say what the search found, for anything that draws results.
///
/// This is what puts the hits in the message list. `Feed::apply` handles it by
/// calling `show_results`, so the ids go out once here and the list, its count
/// and its paging all follow from that — no second path, and no call from this
/// module into a widget.
///
/// Broadcast rather than a direct call on purpose: every route to a search —
/// the box, a saved query, a command — lands in one place, and the list is not
/// the only thing that may want to know.
fn announce(events: &postio_core::bridge::EventSink, query: &ParsedQuery, results: &SearchResults) {
    events.emit(Event::SearchResults {
        query: query.input().to_owned(),
        messages: results.hits.iter().map(|hit| hit.message_id).collect(),
        took: results.elapsed,
    });
}

/// Join the result list to the surfaces around it.
///
/// Three things that hits reaching the list does not do on its own, all of
/// them this crate's because each needs both halves — the `Feeds` that owns
/// the list and the `View` that owns the preview:
///
/// * the column header counts results rather than naming a folder,
/// * the cursor moving through them moves the preview,
/// * `Esc` puts the mailbox back, where it was.
fn install_results(
    window: &Window,
    feeds: &Feeds,
    view: &View,
    held: Held,
    wiring: &Wiring,
    order: Order,
) {
    let list = window.list();
    let finder = window.finder();

    // What the header and the scroller were showing before the results took
    // the list, so `Esc` can put both back. Captured on the way *in* rather
    // than read on the way out: by then the list is the result set, and its
    // offset is the one the user scrolled through the hits to.
    let restore: Rc<std::cell::RefCell<Option<(String, u32, f64)>>> =
        Rc::new(std::cell::RefCell::new(None));

    feeds.messages.connect_results({
        let list = list.clone();
        let restore = restore.clone();
        let order = order.clone();
        move |count| {
            // Only the first result set of a search remembers. Retyping
            // without leaving replaces the hits, and recording *those* as the
            // thing to go back to is how `Esc` ends up returning to a search.
            if restore.borrow().is_none() {
                restore.replace(Some((
                    list.mailbox_name(),
                    list.unread(),
                    list.scroll_offset(),
                )));
            }
            // Canvas 2b: the column says what it is showing. "14 results",
            // not the folder it has stopped listing.
            list.set_mailbox(&results_label(count), 0);
            // And the order those results are in (#499): ranked results
            // labelled `Newest ▾` read as a broken sort.
            list.set_result_order(Some(order.get()));
        }
    });

    // The preview follows the keyboard. `set_focused` wants the whole hit —
    // the snippet and the sender come from the index, not from the row — so
    // the cursor's id is looked up in the results the run held.
    list.cursor().connect_selected_notify({
        let list = list.clone();
        let view = view.clone();
        let feeds = feeds.clone();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |_| {
            if !feeds.messages.showing_results() {
                return;
            }
            let Some(id) = list.cursor_id() else {
                return;
            };
            let held = held.borrow();
            let Some(hit) = held
                .as_ref()
                .and_then(|results| results.hits.iter().find(|hit| hit.message_id == id))
            else {
                return;
            };
            view.set_focused(Some(hit));
            preview(&view, hit, &database, &runtime);
        }
    });

    // `Esc`. The box is dismissed and the folder comes back, because the
    // results are what the box put there.
    finder.connect_dismissed({
        let list = list.clone();
        let feeds = feeds.clone();
        let order = order.clone();
        move || {
            if !feeds.messages.close_results() {
                return;
            }
            // The next search starts ranked, whatever this one was switched
            // to: `Relevance` is the default because it is the answer the
            // ranking exists to give, and a sticky `Newest` would quietly
            // turn search into a date filter for ever after.
            order.set(postio_search::ResultOrder::default());
            list.set_result_order(None);
            let Some((name, unread, offset)) = restore.replace(None) else {
                return;
            };
            list.set_mailbox(&name, unread);
            // After the count is back, not before: `close_results` puts the
            // list at the mailbox's length in the same turn, and an offset
            // set against a scroller still the result set's height would be
            // clamped to the wrong place.
            list.set_scroll_offset(offset);
        }
    });
}

/// What the list column calls a result set.
///
/// Singular is worth the branch: "1 results" is the kind of thing that makes
/// an interface feel unfinished, and this sits at the top of the pane.
fn results_label(count: u32) -> String {
    match count {
        1 => "1 result".to_string(),
        count => format!("{count} results"),
    }
}

/// Resolve `cid:` parts, and open what the preview asks to open.
fn install_preview(view: &View, wiring: &Wiring) {
    let preview = view.preview();
    preview.set_blob_source(crate::reading::cid_source(
        {
            // The preview and the reading pane have the same problem and
            // different notions of "the message on screen", which is why the
            // shared helper takes a closure rather than a widget.
            let preview = preview.clone();
            move || preview.focused()
        },
        wiring.database.clone(),
        wiring.blobs.clone(),
    ));
    install_open(&preview, wiring.commands.clone());
}

/// `Enter` on a previewed result opens it in the reader.
fn install_open(preview: &postio_gtk::search::Preview, commands: CommandSender) {
    preview.connect_open(move |message| {
        if commands
            .send(Command::OpenMessage {
                message: Some(message),
            })
            .is_err()
        {
            tracing::debug!("the runtime has stopped and did not open that");
        }
    });
}

/// Give `@` the account's correspondents.
///
/// The whole list, not a prefix query: the matcher is a subsequence one, so
/// `gh` has to reach `Grace Hopper` and no SQL `LIKE` will find that. Read off
/// the UI thread because it is the one read here whose size is set by the
/// mailbox rather than by the query, and a window that paused at startup to
/// count someone's correspondents would be paying the whole cost up front.
fn load_contacts(finder: &Finder, account: AccountId, wiring: &Wiring) {
    let answer = ask(&wiring.database, &wiring.runtime, move |connection| {
        ContactRepository::new(connection)
            .search(Some(account), "", CONTACT_LIMIT)
            .map_err(|error| tracing::warn!(%error, "could not read the correspondents"))
            .ok()
    });
    glib::spawn_future_local({
        let finder = finder.clone();
        async move {
            let Ok(Some(contacts)) = answer.recv().await else {
                return;
            };
            tracing::debug!(count = contacts.len(), "correspondents read");
            finder.set_contacts(&contacts);
        }
    });
}

/// How many correspondents `@` can offer.
///
/// A bound rather than a page: the matcher needs the whole list to subsequence
/// over. Distinct correspondents are bounded by the people who have written to
/// the account — thousands, against millions of messages — and the palette
/// draws only its own first rows, so this exists to stop a pathological store
/// rather than to page a normal one.
const CONTACT_LIMIT: u32 = 50_000;

#[cfg(test)]
mod tests {
    //! Cutting a result's excerpt, which SQLite no longer does for us (#408).
    //!
    //! Nothing here needs a display: `run` takes a connection and a blob
    //! store and hands back results, which is the whole of the seam.

    use super::*;
    use postio_search::facets::Scope;
    use postio_storage::repository::MessageRepository;
    use postio_storage::test_support;

    /// A store with one message whose body is on disk, indexed the way the
    /// backfill indexes it.
    fn a_message_with_a_body(
        body: &str,
    ) -> (postio_storage::test_support::TempDatabase, AccountId) {
        let database = test_support::temp();
        let connection = database.connection().expect("checkout");
        postio_index::index::ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);

        let mut message = postio_model::Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some("Weekly notes".to_owned());
        message.sync.body_state = postio_model::BodyState::Full;
        let messages = MessageRepository::new(&connection);
        messages.create(&mut message).expect("create");

        messages
            .set_body(
                message.id,
                &postio_storage::repository::StoredBody {
                    text: Some(body.to_owned()),
                    html: None,
                    headers: None,
                },
                postio_model::BodyState::Full,
            )
            .expect("store the body");
        postio_index::index::index_body(&connection, message.id.get(), Some(body))
            .expect("index it");
        drop(connection);
        (database, account.id)
    }

    fn search_for(
        database: &postio_storage::test_support::TempDatabase,
        account: AccountId,
        text: &str,
    ) -> SearchResults {
        let connection = database.connection().expect("checkout");
        let query = postio_search::parse(text, chrono::Utc::now().date_naive());
        run(
            &connection,
            account,
            &query,
            Scope::AllMail,
            postio_search::ResultOrder::Relevance,
        )
        .expect("a search")
    }

    #[test]
    fn a_hit_gets_an_excerpt_cut_from_the_body_it_matched_in() {
        let body = "Dear Ada,\n\nThe difference engine's seventh column is \
                    finished and the drawings are with the printer.\n";
        let (database, account) = a_message_with_a_body(body);

        let results = search_for(&database, account, "printer");

        assert_eq!(results.hits.len(), 1);
        let marked = postio_search::highlight::from_snippet(&results.hits[0].snippet);
        assert_eq!(
            marked
                .matches
                .iter()
                .map(|range| &marked.text[range.clone()])
                .collect::<Vec<_>>(),
            vec!["printer"],
            "snippet: {:?}",
            results.hits[0].snippet
        );
    }

    #[test]
    fn what_is_marked_is_a_word_the_query_actually_matched() {
        // The criterion ADR 0017 named as the thing that would falsify the
        // contentless decision: a highlight regenerated from the blob that
        // points at different words than FTS5 scored is worse than none.
        //
        // `maildir` contains `mail`, and a highlighter that searched for
        // substrings would paint it — for a query that did not match this
        // message on that word at all, because FTS5 tokenizes `maildir` as
        // one token.
        let (database, account) = a_message_with_a_body("the maildir is rebuilt nightly");

        assert!(
            search_for(&database, account, "mail").hits.is_empty(),
            "the query does not match, so there is nothing to highlight"
        );

        let results = search_for(&database, account, "maildir");
        let marked = postio_search::highlight::from_snippet(&results.hits[0].snippet);
        assert_eq!(
            marked
                .matches
                .iter()
                .map(|range| &marked.text[range.clone()])
                .collect::<Vec<_>>(),
            vec!["maildir"]
        );
    }

    #[test]
    fn a_query_with_nothing_to_point_at_leaves_the_snippet_alone() {
        // A structured-only query — `is:unread`, `in:archive` — has no term
        // to mark, and every hit's snippet stays empty exactly as it did when
        // SQLite was cutting them.
        let (database, account) = a_message_with_a_body("anything at all");

        let results = search_for(&database, account, "is:unread");

        assert_eq!(results.hits.len(), 1);
        assert!(results.hits[0].snippet.is_empty());
    }

    #[test]
    fn a_hit_whose_body_is_not_on_this_machine_gets_no_excerpt_rather_than_a_wrong_one() {
        // The message matched on its subject; its body is still on the
        // server. An excerpt cut from nothing would be an empty line that
        // looks like a body with no match in it.
        let database = test_support::temp();
        let connection = database.connection().expect("checkout");
        postio_index::index::ensure_schema(&connection).expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection);
        let mut message = postio_model::Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some("The printer is fixed".to_owned());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("create");
        drop(connection);

        let results = search_for(&database, account.id, "printer");

        assert_eq!(results.hits.len(), 1, "the subject still matched");
        assert!(results.hits[0].snippet.is_empty());
    }
}
