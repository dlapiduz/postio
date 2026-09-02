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
use postio_core::{Command, Event};
use postio_gtk::feed::Feeds;
use postio_gtk::finder::Finder;
use postio_gtk::search::{Outcome, View};
use postio_gtk::window::Window;
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::ids::AccountId;
use postio_search::facets::{Facets, Scope};
use postio_search::{ParsedQuery, SearchResults};
use postio_storage::repository::ContactRepository;
use postio_storage::{Database, PooledConnection};

use crate::Wiring;

/// How many hits one run brings back.
///
/// Not how many *matched* — that is `SearchResults::total_hits`, which the
/// readout draws and which counts far past this. This is the page the preview
/// and, once `postio-1ag`'s follow-up lands, the result list are drawn from,
/// and nobody scrolls two hundred results looking for the one they meant.
const HIT_LIMIT: u32 = 200;

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

    // The column's footer names keys, and the window cannot reach it from
    // `apply_keymap` -- this view is the composition root's, not the
    // window's. So it listens instead, and a rebind reaches the footer the
    // same moment it reaches the keyboard (#828).
    window.connect_keymap({
        let panel = view.panel();
        move |keymap| panel.set_keymap(keymap)
    });

    // The hits the surfaces are drawn from, shared between the run that
    // produces them and the cursor that walks them.
    let held: Held = Rc::new(std::cell::RefCell::new(None));

    // Which order the result set is in (#499). Owned here, beside the scope,
    // because the same run reads both: the executor is asked for ranked or
    // date order per request, and the list header reports whichever the
    // rows are actually in.
    let order: Order = Rc::new(std::cell::Cell::new(postio_search::ResultOrder::default()));

    install_leave_to_list(window, &finder);
    install_preview(&view, wiring, window);
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

/// Answer [`CommandId::ToggleResultOrder`](postio_core::CommandId::ToggleResultOrder)
/// — `o` over results, or a click on
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
/// `work` runs on the blocking pool with a connection of its own — checked
/// out ahead of background work already queued for one
/// (`Database::connection_interactive`), because every caller here is a
/// person waiting on the answer: a search, a reading-pane body. Without that
/// priority this queued behind a first sync's backfill on the same pool
/// `Database::connection` draws from, which is #672 — #425 gave writes a
/// queue with a priority in it and never touched this. `None` from `work` —
/// or a connection that could not be checked out — reaches the caller as
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
            .connection_interactive()
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

/// How many hits get an excerpt cut for them.
///
/// Several screens' worth, and short of [`HIT_LIMIT`] on purpose. Each one
/// costs a blob read, and a person who scrolls past fifty results without
/// refining the query is doing something a snippet was not going to help
/// with. Past this the row falls back to the message's own preview, which it
/// already has.
const SNIPPET_HITS: usize = 50;

/// One search against the index, with an excerpt cut for each hit.
fn run(
    connection: &PooledConnection,
    account: AccountId,
    query: &ParsedQuery,
    scope: Scope,
    order: postio_search::ResultOrder,
) -> Option<SearchResults> {
    let mut results = search(
        connection,
        &SearchRequest {
            // `Account`, not `Unified`, and deliberately: the composition
            // root still opens exactly one account (`first_account`), so
            // searching every account and searching this one are the same
            // set. #186 gave the executor the capability; #183 is what gives
            // the application more than one account to point it at, and
            // switching this to follow `AppState.scope` belongs there — where
            // there is something to observe the difference.
            account: AccountScope::Account(account),
            query,
            scope,
            limit: HIT_LIMIT,
            order,
        },
        chrono::Utc::now(),
    )
    .map_err(|error| tracing::warn!(%error, "the search did not run"))
    .ok()?;
    snippet_hits(connection, query, &mut results);
    Some(results)
}

/// Cuts each hit's excerpt out of its own body text.
///
/// # Why this is here and not in the executor
///
/// The boundary #408 settled. `snippet()` was an FTS5 function over indexed
/// content and the body index has none — `message_bodies_fts` is
/// `content = ''`, which is the point of it (#407). Reconstructing an
/// excerpt needs the body, the body is in the blob store, and `postio-index`
/// is a rusqlite-only leaf that `check-crate-boundaries.py` keeps that way.
/// So the excerpt is cut where both halves are already in hand: here, in the
/// composition root, which is holding a connection and a blob store anyway.
///
/// # Why it agrees with what matched
///
/// `postio_index::index::indexable_text` is the function the *indexer* uses
/// to decide what a message's searchable text is — `text/plain` when there is
/// one, the HTML rendered to text otherwise. Calling the same function means
/// the string highlighted is the string that was indexed, rather than a
/// second guess at it, and `postio_search::highlight`'s token rule is FTS5's
/// own. A message with no local body gets no excerpt rather than a wrong one.
fn snippet_hits(connection: &PooledConnection, query: &ParsedQuery, results: &mut SearchResults) {
    let terms = postio_search::highlight::terms(query);
    if terms.is_empty() {
        // A structured-only query — `is:unread`, `in:archive` — has nothing to
        // point at, and every hit's snippet stays empty exactly as it did
        // when SQLite was cutting them.
        return;
    }
    for hit in results.hits.iter_mut().take(SNIPPET_HITS) {
        let body = crate::compose::load_body(connection, hit.message_id);
        if let Some(text) = postio_index::index::indexable_text(&body) {
            hit.snippet = postio_search::highlight::snippet(&text, &terms);
        }
    }
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
fn install_preview(view: &View, wiring: &Wiring, window: &Window) {
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
    install_open(&preview, window);
}

/// `Enter` on a previewed result opens it in the reader.
///
/// Through `Window::act` rather than straight onto the command bus (#767).
/// The bus owns the verbs that *write* — archive, flag, move, snooze — and
/// opening a message writes nothing; it moves the cursor to the result and
/// lets the reading pane fill the way it does for any other landing, which
/// is also what takes the pane back from the preview.
///
/// Sending it to the bus was the bug: nothing there answered `OpenMessage`,
/// so the dispatcher rejected it and the one gesture whose whole purpose is
/// "open this" did nothing at all.
fn install_open(preview: &postio_gtk::search::Preview, window: &Window) {
    preview.connect_open(glib::clone!(
        #[weak]
        window,
        move |message| {
            window.act(Command::OpenMessage {
                message: Some(message),
            });
        }
    ));
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

#[cfg(test)]
mod interactive_read {
    //! `ask` does not queue behind a backfill on the same pool. #672.
    //!
    //! `postio-storage/tests/connection_priority.rs` proves the property
    //! underneath this without an engine at all, the same way
    //! `postio-storage/tests/write_gate.rs` underlies #425's
    //! `postio-session/tests/interactive_write.rs`. This is that end-to-end
    //! claim for the read side: the engine is real, its backend is
    //! `postio_imap::backend::MockBackend` (the seam CLAUDE.md names for
    //! exactly this), and the pool is left at its ordinary size — see
    //! `engine_over` for why shrinking it is the wrong way to reproduce
    //! exhaustion here.
    //!
    //! # Why the pool still ends up fully exhausted
    //!
    //! Two mailboxes and room for two sync lanes means the engine syncs both
    //! at once, and its wave holds both connections until the whole wave —
    //! not each mailbox — finishes; that is exactly
    //! [`postio_runtime::engine`]'s own `RESERVED_FOR_ELSEWHERE`, the two
    //! connections a sync wave leaves for "the UI thread's reads" and "the
    //! engine's own housekeeping between waves". This test claims that
    //! reserve for itself instead, to stand in for whatever else in a real
    //! session would be reading with no particular urgency — an unrelated
    //! mailbox open in the list, an idle poll. What #672 fixes is which of
    //! the two waiters that then queue for the one connection that frees
    //! goes first.

    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use postio_core::bridge::event_channel;
    use postio_imap::backend::{MockBackend, MockMailbox, MockMessage};
    use postio_model::ids::MessageId;
    use postio_runtime::Engine;
    use postio_runtime::engine::{EngineParts, NetworkSource, SystemClock};
    use postio_storage::repository::{
        AccountRepository, ListQuery, ListScope, MailboxRepository, MessageRepository,
    };
    use postio_storage::{BlobStore, Database, test_support};

    /// Long enough that a thread which has announced it is about to block
    /// really is blocked by the time the next step runs.
    const ENOUGH_TO_BLOCK: Duration = Duration::from_millis(50);

    const BULK: &str = "Lists";
    const BULK_MESSAGES: u32 = 2_000;

    fn message(n: u32) -> Vec<u8> {
        format!(
            "From: Ada Lovelace <ada@example.com>\r\n\
             To: Postio <postio@example.net>\r\n\
             Subject: message {n}\r\n\
             Message-ID: <m-{n}@example.com>\r\n\
             Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
             \r\n\
             Body {n}.\r\n"
        )
        .into_bytes()
    }

    fn folder(path: &str, messages: u32) -> MockMailbox {
        let mut mailbox = MockMailbox::new(path);
        for n in 1..=messages {
            mailbox = mailbox.message(MockMessage::new(message(n)));
        }
        mailbox
    }

    /// A database at the pool's ordinary size, an engine backfilling
    /// `backend` over it, and the directories that have to outlive both.
    ///
    /// The default size rather than something smaller: the engine's own
    /// discovery and housekeeping reads share this pool with its sync lanes,
    /// and sizing it down to force exhaustion risks starving *that* instead
    /// of exercising #672. The test creates its own exhaustion later, on
    /// purpose, by holding connections itself.
    fn engine_over(backend: Arc<MockBackend>) -> (Database, Engine, tempfile::TempDir) {
        let directory = tempfile::tempdir().expect("a database directory");
        let database = Database::open_with(
            directory.path().join("postio.db"),
            &test_support::key(),
            postio_storage::db::DEFAULT_MAX_CONNECTIONS,
        )
        .expect("a database");
        let account = {
            let connection = database.connection().expect("a connection");
            test_support::account(&connection)
        };
        let blobs_directory = tempfile::tempdir().expect("a blob directory");
        let blobs = BlobStore::open(
            blobs_directory.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let (sink, _events) = event_channel();

        let engine = Engine::spawn(EngineParts {
            account: account.id,
            database: database.clone(),
            blobs,
            backend,
            smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
            tokens: Arc::new(postio_imap::auth::StoredPasswordSource::new(Arc::new(
                postio_imap::secret::MemorySecretStore::default(),
            ))),
            events: sink,
            retry: Default::default(),
            backfill: Default::default(),
            reconnect: Default::default(),
            watch: Default::default(),
            network: NetworkSource::Ignored,
            mailbox_roles: Default::default(),
            clock: Arc::new(SystemClock),
        })
        .expect("the engine starts");

        (database, engine, directory)
    }

    /// How many messages the store holds under `path`, or `0` before the
    /// mailbox itself has arrived.
    fn stored(database: &Database, path: &str) -> u32 {
        let Ok(connection) = database.connection() else {
            return 0;
        };
        let Ok(accounts) = AccountRepository::new(&connection).list() else {
            return 0;
        };
        let Some(account) = accounts.into_iter().next() else {
            return 0;
        };
        let Ok(mailboxes) = MailboxRepository::new(&connection).list_for_account(account.id) else {
            return 0;
        };
        let Some(mailbox) = mailboxes.into_iter().find(|mailbox| mailbox.path == path) else {
            return 0;
        };
        MessageRepository::new(&connection)
            .count(&ListQuery {
                scope: ListScope::Mailbox(mailbox.id),
                limit: 0,
                after: None,
            })
            .unwrap_or(0)
    }

    /// The id of the first message under `path`, once there is one.
    fn first_message(database: &Database, path: &str) -> Option<MessageId> {
        let connection = database.connection().ok()?;
        let accounts = AccountRepository::new(&connection).list().ok()?;
        let account = accounts.into_iter().next()?;
        let mailbox = MailboxRepository::new(&connection)
            .list_for_account(account.id)
            .ok()?
            .into_iter()
            .find(|mailbox| mailbox.path == path)?;
        MessageRepository::new(&connection)
            .page(&ListQuery {
                scope: ListScope::Mailbox(mailbox.id),
                limit: 1,
                after: None,
            })
            .ok()?
            .into_iter()
            .next()
            .map(|row| row.id)
    }

    /// Waits for `condition`, or gives up and says what was true when it did.
    ///
    /// A liveness bound and nothing else, deliberately enormous for the
    /// reason `postio-runtime/tests/sync_wave.rs` sets out: a deadline small
    /// enough to be a performance budget is a flake waiting for a loaded
    /// machine.
    async fn until(what: &str, mut condition: impl FnMut() -> bool) {
        let waited = tokio::time::timeout(Duration::from_secs(180), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(waited.is_ok(), "timed out waiting for {what}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reading_pane_body_load_does_not_wait_for_the_backfill() {
        let backend = Arc::new(
            MockBackend::builder()
                .mailbox(folder("INBOX", 3))
                .mailbox(folder(BULK, BULK_MESSAGES))
                .build(),
        );
        backend.set_latency(Duration::from_millis(20));

        let (database, engine, _directory) = engine_over(backend);

        until("INBOX to fully arrive", || stored(&database, "INBOX") >= 3).await;
        let message = first_message(&database, "INBOX").expect("a message in the inbox");

        // Mid-backfill, and demonstrably so: the bulk folder has begun
        // arriving and is nowhere near done. Its lane now holds one of the
        // pool's connections for as long as the whole folder takes — INBOX
        // finished above and gave its own back.
        until("the bulk backfill to be under way", || {
            stored(&database, BULK) > 100
        })
        .await;

        // Exhaust the pool the rest of the way. Two mailboxes with room for
        // two lanes means the engine's own sync wave holds both INBOX's and
        // BULK's connections until the *wave* finishes, not just until each
        // mailbox does — so it is still holding two, not one, long after
        // INBOX's three messages are in. What is left is exactly
        // `postio_runtime::engine`'s own `RESERVED_FOR_ELSEWHERE`, and these
        // two connections stand in for what it is reserved for: the "an
        // unrelated mailbox open in the list, an idle poll" the module docs
        // describe. Held on its own thread with a bound: each `get()` waits
        // for whatever the engine has not claimed, and if that is somehow
        // never enough this fails loudly rather than hanging the suite.
        let (spare_sender, spare_receiver) = mpsc::channel();
        let pool_for_spares = database.pool().clone();
        let spare_count = 2;
        std::thread::spawn(move || {
            let held: Vec<_> = (0..spare_count)
                .map(|_| pool_for_spares.get().expect("a connection eventually"))
                .collect();
            let _ = spare_sender.send(held);
        });
        let mut held = spare_receiver
            .recv_timeout(Duration::from_secs(60))
            .expect("could not exhaust the pool's spare capacity in time");
        assert_eq!(
            database.pool().idle_connections(),
            0,
            "the setup is supposed to leave nothing idle"
        );

        let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));

        // A plain background read asks first...
        let (announced, arrived) = mpsc::channel();
        let pool = database.pool().clone();
        let order_for_background = Arc::clone(&order);
        let background = std::thread::spawn(move || {
            announced.send(()).expect("the test is listening");
            let _connection = pool.get().expect("a connection eventually");
            order_for_background.lock().unwrap().push("background");
        });
        arrived.recv().expect("the background thread starts");
        std::thread::sleep(ENOUGH_TO_BLOCK);

        // ...and the reading pane's body load asks second.
        let runtime = tokio::runtime::Handle::current();
        let order_for_interactive = Arc::clone(&order);
        let answer = crate::search::ask(&database, &runtime, move |connection| {
            // Recorded here, the instant the connection is in hand, rather
            // than after `answer.recv().await` below: that round trip adds a
            // channel send and a task wake on top of the checkout itself, so
            // timing the *order's* answer would measure that extra latency
            // instead of which of the two actually got a connection first.
            order_for_interactive.lock().unwrap().push("interactive");
            MessageRepository::new(connection)
                .get(message)
                .ok()
                .flatten()
        });
        // Observable rather than slept on: the interactive checkout counts
        // itself as waiting before it blocks, which is exactly what the
        // background checkout has to be able to see.
        let interactive_wait = tokio::time::timeout(Duration::from_secs(10), async {
            while !database.pool().interactive_is_waiting() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await;
        assert!(
            interactive_wait.is_ok(),
            "the interactive checkout never registered as waiting"
        );

        // Only one: the rest stay held, so the connection that frees is the
        // single contested slot the background thread and the interactive
        // read are both already queued for — not one each.
        drop(held.pop().expect("at least one spare was held"));
        let result = tokio::time::timeout(Duration::from_secs(10), answer.recv())
            .await
            .expect("the interactive read did not complete in time")
            .expect("an answer");
        background.join().expect("the background checkout finishes");

        assert!(
            result.is_some(),
            "the reading pane did not find the message it asked for"
        );
        assert_eq!(
            *order.lock().unwrap(),
            vec!["interactive", "background"],
            "the backfill was already holding one connection and a plain \
             background read was already queued for the other, and the \
             reading-pane load got it last anyway. That is #672: a read a \
             person is waiting on has to overtake background work that got \
             there first, or a first sync locks the reading pane out for as \
             long as it holds every connection."
        );

        engine.stop();
    }
}
