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
use postio_core::bridge::CommandSender;
use postio_core::{Command, Event};
use postio_gtk::feed::Feeds;
use postio_gtk::finder::Finder;
use postio_gtk::search::{Outcome, View};
use postio_gtk::window::Window;
use postio_index::{SearchRequest, search};
use postio_model::ids::AccountId;
use postio_search::facets::{Facets, Scope};
use postio_search::{ParsedQuery, SearchResults};
use postio_storage::repository::ContactRepository;
use postio_storage::{BlobStore, Database, PooledConnection};

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

    // The hits the surfaces are drawn from, shared between the run that
    // produces them and the cursor that walks them.
    let held: Held = Rc::new(std::cell::RefCell::new(None));

    install_preview(&view, wiring);
    install_run(&view, &finder, account.id, wiring, held.clone());
    install_results(window, feeds, &view, held, wiring);
    load_contacts(&finder, account.id, wiring);

    Some(view)
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
fn install_run(view: &View, finder: &Finder, account: AccountId, wiring: &Wiring, held: Held) {
    let Some(live) = finder.live() else {
        // The readout is built by `Finder::attach`, which the window does
        // before this runs. If it is ever missing, search silently does
        // nothing — which is the bug this module exists to fix, so it is worth
        // a line rather than a `return` nobody sees.
        tracing::error!("the search box has no readout; nothing will answer a query");
        return;
    };

    let database = wiring.database.clone();
    let blobs = wiring.blobs.clone();
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
            let hits = ask(&database, &runtime, {
                let query = query.clone();
                move |connection| run(connection, account, &query, scope)
            });

            glib::spawn_future_local({
                let live = live.clone();
                let view = view.clone();
                let database = database.clone();
                let blobs = blobs.clone();
                let runtime = runtime.clone();
                let events = events.clone();
                let held = held.clone();
                async move {
                    let Ok(Some(results)) = hits.recv().await else {
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
                    focus(&view, &results, &database, &blobs, &runtime);
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

/// One search against the index.
fn run(
    connection: &PooledConnection,
    account: AccountId,
    query: &ParsedQuery,
    scope: Scope,
) -> Option<SearchResults> {
    search(
        connection,
        &SearchRequest {
            account_id: account,
            query,
            scope,
            limit: HIT_LIMIT,
        },
        chrono::Utc::now(),
    )
    .map_err(|error| tracing::warn!(%error, "the search did not run"))
    .ok()
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
                    account_id: account,
                    query: &query,
                    scope,
                    limit: HIT_LIMIT,
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
/// [`follow_cursor`] takes over from the next keystroke on. Both go through
/// [`preview`], so there is one path to the pane rather than two that can
/// disagree.
fn focus(
    view: &View,
    results: &SearchResults,
    database: &Database,
    blobs: &BlobStore,
    runtime: &tokio::runtime::Handle,
) {
    view.set_focused(results.hits.first());
    let Some(hit) = results.hits.first() else {
        return;
    };
    preview(view, hit, database, blobs, runtime);
}

/// Draw `hit`'s body into the preview.
fn preview(
    view: &View,
    hit: &postio_search::SearchHit,
    database: &Database,
    blobs: &BlobStore,
    runtime: &tokio::runtime::Handle,
) {
    // The snippet is already on screen — highlighted, from the index — so this
    // is the body arriving under it rather than the pane waiting on a blob
    // read to show anything at all.
    let message = hit.message_id;
    let sender = hit.from.as_ref().map(|from| from.address.clone());
    let answer = ask(database, runtime, {
        let blobs = blobs.clone();
        move |connection| Some(crate::compose::load_body(connection, &blobs, message))
    });
    glib::spawn_future_local({
        let view = view.clone();
        async move {
            let Ok(Some(body)) = answer.recv().await else {
                return;
            };
            let preview = view.preview();
            // The focus may have moved on while the blob was read. Painting a
            // body into a preview showing a different message would be worse
            // than leaving the snippet alone.
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
fn install_results(window: &Window, feeds: &Feeds, view: &View, held: Held, wiring: &Wiring) {
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
        let blobs = wiring.blobs.clone();
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
            preview(&view, hit, &database, &blobs, &runtime);
        }
    });

    // `Esc`. The box is dismissed and the folder comes back, because the
    // results are what the box put there.
    finder.connect_dismissed({
        let list = list.clone();
        let feeds = feeds.clone();
        move || {
            if !feeds.messages.close_results() {
                return;
            }
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
