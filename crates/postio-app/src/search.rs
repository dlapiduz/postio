//! Search, joined to the store's owner.
//!
//! `postio-gtk` built the whole search surface and cannot run a search: it may
//! not link SQLite, so the box paces the query and then hands it to whoever
//! owns the store -- the host, asked through this crate's client (ADR 0041).
//! Everything here is the other half of a seam `postio-gtk` deliberately left
//! open — [`Live::connect_run`], [`View::set_facets`], [`View::set_focused`],
//! [`Finder::set_contacts`] — and until it existed, typing in the box did
//! nothing at all (`postio-1ag`).
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
//! The GTK main loop must never be inside a query. Every read is a
//! [`Client`] call: the host runs it on its own runtime and the answer comes
//! back through a oneshot this loop can wait on, one call where there was one
//! read (`Req::SearchHits`, `Req::Facets`, `Req::StoredBody`).
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
use postio_gtk::feed::{Feeds, Folders};
use postio_gtk::finder::Finder;
use postio_gtk::search::{Outcome, View};
use postio_gtk::window::Window;
use postio_model::AccountScope;
use postio_model::ids::AccountId;
use postio_search::facets::{Facets, Scope};
use postio_search::{ParsedQuery, SearchResults};

use crate::Wiring;
use crate::settings_accounts::Reindexing;
// The search itself -- `postio_session::search`, one executor for every
// frontend (#660) -- runs in the host now, behind `Client::search_hits` and
// `Client::facets`.
use postio_client::Client;

/// Read the store on the runtime and answer over a channel: moved to the one
/// surface that still reads the store itself, and named here for the test
/// below that proves it does not queue behind a backfill (#672).
#[cfg(test)]
pub(crate) use crate::orientation::ask;

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
///
/// `client` is the window's, the same one the list and the reading pane
/// read through: every read here is one of its calls.
pub async fn install(
    window: &Window,
    wiring: &Wiring,
    client: Client,
    feeds: &Feeds,
    reindexing: Reindexing,
) -> Option<View> {
    install_for(window, &wiring.events, client, feeds, reindexing).await
}

/// [`install`], for a window whose store's owner may be another process:
/// `events` is where the window hears a search's own sentences.
pub async fn install_for(
    window: &Window,
    events: &postio_core::bridge::EventSink,
    client: Client,
    feeds: &Feeds,
    reindexing: Reindexing,
) -> Option<View> {
    // `first_account`'s answer, asked of the host: the first enabled account
    // in creation order, which is the order it lists them in.
    let account = client
        .accounts()
        .await
        .map_err(|error| tracing::error!(%error, "cannot read the accounts: {error}"))
        .ok()?
        .into_iter()
        .find(|account| account.enabled)?;
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
    install_preview(&view, &client, window).await;
    install_run(
        &view,
        &finder,
        window,
        feeds,
        events,
        &client,
        held.clone(),
        order.clone(),
        reindexing,
    )
    .await;
    install_scope_rerun(window, &finder);
    install_results(window, feeds, &view, held, &client, order.clone()).await;
    install_order_toggle(window, &finder, feeds, order);
    load_contacts(&finder, account.id, &client).await;
    load_labels(&finder, account.id, &client).await;

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
    // Weak, for the reason `install_run` states below and this function did
    // not follow: the window owns the finder that owns these handlers, so a
    // strong clone is a cycle and the window never frees (#1072, and the
    // three #794 catalogued).
    //
    // A window that has gone is not a focus destination, so an upgrade that
    // fails means there is nothing left to do rather than something to
    // report. `connect_tab` answers `false` in that case — it did not claim
    // the keyboard — which is the honest answer and lets any later handler
    // try, exactly as if this one had never been registered.
    let weak = glib::object::ObjectExt::downgrade(window);
    finder.connect_search({
        let weak = weak.clone();
        move |_parsed| {
            if let Some(window) = weak.upgrade() {
                window.list().grab_focus();
            }
        }
    });
    finder.connect_tab({
        move || match weak.upgrade() {
            Some(window) => {
                window.list().grab_focus();
                true
            }
            None => false,
        }
    });
}

/// Which accounts a search under `scope` could not reach, by the name the
/// sidebar shows, in the sidebar's order.
///
/// ADR 0005 Q10: *a view that cannot include an account says so, names the
/// account, and stays usable.* The list obeys that rule through
/// `list_state::derive_aggregate`; a search is the other surface that can
/// answer for less mail than the user thinks they asked about, and its answer
/// is a number, which looks just as complete either way.
///
/// # Why the composition root and not the executor
///
/// Which accounts answered is a fact about *connections*. `postio-index` only
/// ever sees the store, and a search of a store whose account is offline
/// reads exactly like one whose account is fine — the rows are all local.
/// This is the one layer holding both halves.
///
/// # One rule, two populations
///
/// `list_state::is_current` is the rule, and it is shared with
/// `Window::set_folders`' own reach calculation rather than re-stated here
/// (#811: the banner and the selection came to disagree about which account
/// was which precisely by deriving separately). What differs is the
/// population, legitimately: the list vouches for the accounts it is
/// *drawing*, and a unified search covers every enabled account whether or
/// not the list is currently showing one.
///
/// A single-account search names nothing: it leaves nothing out, whatever the
/// other accounts are doing. A **disabled** account names nothing either, and
/// for free — the sidebar's list is built from the enabled accounts, so one
/// that is switched off is never in it to be named.
pub(crate) fn unreachable_accounts(
    window: &Window,
    folders: &Folders,
    scope: AccountScope,
) -> Vec<String> {
    if scope.is_single_account() {
        return Vec::new();
    }
    let statuses = folders.statuses();
    window
        .sidebar()
        .account_names()
        .into_iter()
        .filter(|(id, _)| {
            statuses
                .iter()
                .find(|(candidate, _)| candidate == id)
                // Silence is not a claim that a server is reachable: an
                // account nothing has reported on has not answered either.
                .is_none_or(|(_, status)| !postio_gtk::list_state::is_current(status))
        })
        .map(|(_, name)| name)
        .collect()
}

/// Whether an account this search's `scope` covers is rebuilding its local
/// search index right now (#981).
///
/// A single account asks about itself; a unified search asks whether *any*
/// enabled account is mid-rebuild — a coarser answer than `unreachable_accounts`
/// gives, deliberately: `corpus_complete` is already a single boolean over
/// the whole scope rather than a per-account list (unlike `unreachable`,
/// which Q10 asks to name accounts individually), so there is no finer
/// answer to compose it from without growing a second caveat shape.
fn reindexing_covers(reindexing: &Reindexing, scope: AccountScope) -> bool {
    match scope.account() {
        Some(id) => reindexing.borrow().contains(&id),
        None => !reindexing.borrow().is_empty(),
    }
}

/// Ask the query again when the account scope changes.
///
/// The scope is read per run, so a *new* query already follows it — but the
/// query on screen when somebody clicks another account is the one they are
/// looking at, and leaving it is worse than making them retype it: a result
/// list that says "14 hits" for a scope the window is no longer in is an
/// answer to a question nobody is asking (#961).
///
/// Registered alongside the composition root's own scope handler rather than
/// inside it: `Sidebar::connect_scope_selected` pushes, so both run, and the
/// search's reaction stays in the module that owns searching.
///
/// [`postio_gtk::search::Live::rerun`] is a no-op when nothing has been
/// asked, so this costs nothing while the box is closed.
fn install_scope_rerun(window: &Window, finder: &Finder) {
    window.sidebar().connect_scope_selected({
        let finder = finder.clone();
        move |_scope| {
            if let Some(live) = finder.live() {
                live.rerun();
            }
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
    // Weak, for the reason `install_run` states: this handler is stored on
    // the window itself, so a strong clone is a cycle with no third party in
    // it at all -- the window holding a closure holding the window (#1072).
    let weak = glib::object::ObjectExt::downgrade(window);
    window.connect_command({
        let finder = finder.clone();
        let feeds = feeds.clone();
        move |id| {
            if id != postio_core::CommandId::ToggleResultOrder || !feeds.messages.showing_results()
            {
                return;
            }
            let Some(window) = weak.upgrade() else {
                return;
            };
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
/// Carry what [`ask`] answered through the blocking pool, where `then`
/// runs on it, and hand back what that made.
///
/// For work that is CPU rather than I/O -- sanitising a body is two
/// html5ever parses -- and that must be done neither while holding the
/// reader `ask` borrowed nor on the main thread, where every message the
/// cursor settled on used to pay for it.
pub(crate) fn then_off_thread<T, U>(
    runtime: &tokio::runtime::Handle,
    answer: Answer<T>,
    then: impl FnOnce(T) -> U + Send + 'static,
) -> Answer<U>
where
    T: Send + 'static,
    U: Send + 'static,
{
    let (sender, receiver) = async_channel::bounded(1);
    runtime.spawn(async move {
        let Ok(answer) = answer.recv().await else {
            return;
        };
        let made = match answer {
            Some(value) => tokio::task::spawn_blocking(move || then(value)).await.ok(),
            None => None,
        };
        let _ = sender.send(made).await;
    });
    receiver
}

pub(crate) fn ask<T, F, Fut>(
    database: &Store,
    runtime: &tokio::runtime::Handle,
    work: F,
) -> Answer<T>
where
    T: Send + 'static,
    F: FnOnce(Checkout) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Option<T>> + Send,
{
    let (sender, receiver) = async_channel::bounded(1);
    let database = database.clone();
    runtime.spawn(async move {
        // A turn on a warm reader rather than a connection of its own: a
        // search run took four cold caches per keystroke (#1602).
        let answer = match database.read().await {
            Ok(reader) => {
                let answer = work(reader.checkout()).await;
                drop(reader);
                answer
            }
            Err(error) => {
                tracing::warn!(%error, "no connection to read the index with");
                None
            }
        };
        let _ = sender.send_blocking(answer);
    });
    receiver
}

/// What [`ask`] hands back: one answer, or none.
type Answer<T> = async_channel::Receiver<Option<T>>;

/// Run a search when the box says a query is due.
#[allow(clippy::too_many_arguments)]
async fn install_run(
    view: &View,
    finder: &Finder,
    window: &Window,
    feeds: &Feeds,
    events: &postio_core::bridge::EventSink,
    client: &Client,
    held: Held,
    order: Order,
    reindexing: Reindexing,
) {
    let Some(live) = finder.live() else {
        // The readout is built by `Finder::attach`, which the window does
        // before this runs. If it is ever missing, search silently does
        // nothing — which is the bug this module exists to fix, so it is worth
        // a line rather than a `return` nobody sees.
        tracing::error!("the search box has no readout; nothing will answer a query");
        return;
    };

    let client = client.clone();
    let events = events.clone();
    let view = view.clone();
    let folders = feeds.folders.clone();
    // Weak, because the window owns the finder that owns this handler; a
    // strong clone here is a cycle that keeps the window alive for the life
    // of the process.
    let window = glib::object::ObjectExt::downgrade(window);

    live.connect_run({
        let live = live.clone();
        move |parsed, sequence| {
            // Owned, because the read happens on another thread and the box
            // is free to keep typing while it does.
            let query = parsed.clone();
            let scope = view.scope();
            // The account scope, read fresh on this side of the thread hop
            // exactly as the role scope above is. A window that has gone
            // away answers nothing rather than searching every account: a
            // teardown is not a widening.
            let Some(account) = window.upgrade().map(|window| window.scope()) else {
                live.settled(sequence);
                return;
            };
            // Read on this side of the thread hop: the cell lives with the
            // GTK loop, and the value — `Copy` — travels with the work.
            let order = order.get();
            // One excerpt: the preview draws the focused hit's, and only
            // until its body arrives -- `focus` reads that body straight
            // after. The rest were fifty body reads between the keystroke
            // and this answer (#1613).
            let hits = client.clone();
            let hits = {
                let query = query.clone();
                async move { hits.search_hits(account, query, scope, order, 1).await }
            };

            glib::spawn_future_local({
                let live = live.clone();
                let view = view.clone();
                let client = client.clone();
                let events = events.clone();
                let held = held.clone();
                let folders = folders.clone();
                let window = window.clone();
                let reindexing = reindexing.clone();
                async move {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
                    // on its own runtime.
                    let Ok(Some(results)) = hits.await else {
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
                    // Which accounts this answer could not include. Read
                    // here, on the GTK side, and against the scope the search
                    // actually ran under -- the user is free to switch scope
                    // while the store is answering, and a caveat about the
                    // scope they moved to would be about a different question.
                    // Spelled out rather than chained: the read awaits, and
                    // a closure cannot.
                    let unreachable = match window.upgrade() {
                        Some(window) => unreachable_accounts(&window, &folders, account),
                        None => Vec::new(),
                    };
                    // Whether an account this answer covers is rebuilding
                    // its local index right now (#981) -- read against the
                    // same scope the search ran under, for the reason
                    // `unreachable` above is.
                    let reindexing_now = reindexing_covers(&reindexing, account);
                    // The readout first: it is what the field is showing and
                    // what the user is waiting for.
                    if !live.deliver(
                        sequence,
                        Outcome::of(&results)
                            .with_unreachable(unreachable)
                            .with_reindexing(reindexing_now),
                    ) {
                        // Superseded. Everything downstream of these results
                        // is about a question nobody is asking.
                        return;
                    }
                    // Held before it is announced: the event puts the hits in
                    // the list, which moves the cursor, which looks them up
                    // here. Announcing first would race the cursor against the
                    // results it is a cursor into.
                    // The offer rides with the results rather than with the
                    // facet counts: it is computed by the search itself, and
                    // the counts arrive on their own job a moment later. Drawn
                    // before `focus` for the same reason the readout is --
                    // this is what somebody staring at an empty list is
                    // waiting to be told.
                    view.set_suggestion(results.suggestion.as_ref());
                    // And which word the list is for, when the box answered a
                    // word that found nothing with the one that was meant.
                    // Beside the offer, for the same reason: it is what the
                    // list below it means.
                    view.set_instead(results.instead.as_ref());
                    // POSTIO-GLIB-SAFE: nothing under this await wants a reactor. The
                    // network work it reaches is spawned onto the runtime and answers over a
                    // channel -- `onboarding::probe_with_offer` is the shape -- and what is
                    // left is store reads, whose futures this engine makes self-contained.
                    // Measured rather than assumed: `app_suite::glib_main_context` opens a
                    // store and reads it on this context with no runtime anywhere, and fails
                    // loudly if that stops being true.
                    focus(&view, &results, &client).await;
                    held.replace(Some(results));
                    // Scoped, so the borrow is gone before `facets` runs:
                    // nothing downstream needs `held` today, and a borrow
                    // left open across two calls is a `borrow_mut` panic
                    // waiting for whoever edits this next.
                    if let Some(results) = held.borrow().as_ref() {
                        announce(&events, &query, results);
                    }
                    facets(&view, &live, sequence, account, &query, scope, &client)
                        // POSTIO-GLIB-SAFE: nothing under this await wants a reactor. The
                        // network work it reaches is spawned onto the runtime and answers over a
                        // channel -- `onboarding::probe_with_offer` is the shape -- and what is
                        // left is store reads, whose futures this engine makes self-contained.
                        // Measured rather than assumed: `app_suite::glib_main_context` opens a
                        // store and reads it on this context with no runtime anywhere, and fails
                        // loudly if that stops being true.
                        .await;
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
async fn facets(
    view: &View,
    live: &postio_gtk::search::Live,
    sequence: u64,
    account: AccountScope,
    query: &ParsedQuery,
    scope: Scope,
    client: &Client,
) {
    // The scope the hits were counted under, carried rather than re-read:
    // the columns have to describe *this* result set, and the user is free
    // to switch scope while this second round trip is in flight.
    let answer = {
        let client = client.clone();
        let query = query.clone();
        async move { client.facets(account, query, scope).await }
    };
    glib::spawn_future_local({
        let view = view.clone();
        let live = live.clone();
        async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
            // on its own runtime.
            let Ok(Some(facets)) = answer.await else {
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
async fn focus(view: &View, results: &SearchResults, client: &Client) {
    view.set_focused(results.hits.first());
    let Some(hit) = results.hits.first() else {
        return;
    };
    preview(view, hit, client).await;
}

/// Draw `hit`'s body into the preview.
async fn preview(view: &View, hit: &postio_search::SearchHit, client: &Client) {
    // The snippet is already on screen — highlighted, from the index — so this
    // is the body arriving under it rather than the pane waiting on a read to
    // show anything at all.
    let message = hit.message_id;
    let sender = hit.from.as_ref().map(|from| from.address.clone());
    // Judged and sanitised off the main thread, under the policy the preview
    // would draw this sender with, so the main thread only loads it.
    let remote = view.preview().remote_images_for(sender.as_deref());
    let answer = {
        let client = client.clone();
        async move { client.stored_body(message).await }
    };
    glib::spawn_future_local({
        let view = view.clone();
        async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
            // on its own runtime.
            let Ok(body) = answer.await else {
                return;
            };
            let Ok((body, prepared)) = gtk::gio::spawn_blocking(move || {
                let prepared = postio_ui::reader::document::prepare_message(&body, remote);
                (body, prepared)
            })
            .await
            else {
                return;
            };
            let preview = view.preview();
            // The focus may have moved on while the body was read. Painting
            // a body into a preview showing a different message would be
            // worse than leaving the snippet alone.
            if preview.focused() != Some(message) {
                return;
            }
            preview.set_prepared_body(message, &body, sender.as_deref(), Some(prepared));
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
async fn install_results(
    window: &Window,
    feeds: &Feeds,
    view: &View,
    held: Held,
    client: &Client,
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
        // Weak: this handler is registered on `feeds`, which outlives the
        // window, and a strong clone here is the cycle #794 catalogued and
        // `install_run` states the rule against. `window_teardown` is what
        // notices, and it noticed this one.
        let window = glib::object::ObjectExt::downgrade(window);
        let finder = finder.clone();
        move |count| {
            let Some(window) = window.upgrade() else {
                return;
            };
            // The list is showing this query's results, which is the fact
            // `Window::set_searching` wants -- not "there is text in the box",
            // which stays true after `Esc` has put the folder back.
            //
            // Without this the list never learned a search was on, so a query
            // that matched nothing drew the *mailbox's* empty state: "nothing
            // left to triage", over a mailbox with thousands in it.
            window.set_searching(Some(&finder.query().text));
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
        let client = client.clone();
        move |_| {
            postio_session::blocking::now(async {
                if !feeds.messages.showing_results() {
                    return;
                }
                let Some(id) = list.cursor_id() else {
                    return;
                };
                // Cloned out and the borrow dropped before the await. A
                // `RefCell` borrow held across a suspension is a borrow that
                // lasts as long as the future does, and the next GTK callback
                // to reach this cell -- another keystroke, another result set
                // arriving -- panics on it. A `SearchHit` is small.
                let hit = {
                    let held = held.borrow();
                    held.as_ref()
                        .and_then(|results| results.hits.iter().find(|hit| hit.message_id == id))
                        .cloned()
                };
                let Some(hit) = hit else {
                    return;
                };
                view.set_focused(Some(&hit));
                preview(&view, &hit, &client).await;
            })
        }
    });

    // `Esc`. The box is dismissed and the folder comes back, because the
    // results are what the box put there.
    finder.connect_dismissed({
        let list = list.clone();
        let feeds = feeds.clone();
        let order = order.clone();
        // Weak, for the reason `connect_results` above gives.
        let window = glib::object::ObjectExt::downgrade(window);
        move || {
            if !feeds.messages.close_results() {
                return;
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            // The next search starts ranked, whatever this one was switched
            // to: `Relevance` is the default because it is the answer the
            // ranking exists to give, and a sticky `Newest` would quietly
            // turn search into a date filter for ever after.
            order.set(postio_search::ResultOrder::default());
            list.set_result_order(None);
            // The results have left the list, so the list is a mailbox again
            // and its empty state is the mailbox's once more.
            window.set_searching(None);
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
async fn install_preview(view: &View, client: &Client, window: &Window) {
    let preview = view.preview();
    preview.set_blob_source(crate::reading::cid_source(
        {
            // The preview and the reading pane have the same problem and
            // different notions of "the message on screen", which is why the
            // shared helper takes a closure rather than a widget.
            let preview = preview.clone();
            move || preview.focused()
        },
        client.clone(),
    ));
    install_open(&preview, window).await;
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
async fn install_open(preview: &postio_gtk::search::Preview, window: &Window) {
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
///
/// The host reads at most 50,000 of them (`postio_host::compose`'s
/// `CORRESPONDENT_LIMIT`): a bound rather than a page, because the matcher
/// needs the whole list to subsequence over. Distinct correspondents are
/// bounded by the people who have written to the account -- thousands,
/// against millions of messages -- so the bound exists to stop a
/// pathological store rather than to page a normal one.
async fn load_contacts(finder: &Finder, account: AccountId, client: &Client) {
    let client = client.clone();
    glib::spawn_future_local({
        let finder = finder.clone();
        async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
            // on its own runtime.
            let found = client.correspondents(account).await;
            let Ok(contacts) =
                found.map_err(|error| tracing::warn!(%error, "could not read the correspondents"))
            else {
                return;
            };
            tracing::debug!(count = contacts.len(), "correspondents read");
            finder.set_contacts(&contacts);
        }
    });
}

/// Reads the account's labels and hands them to `+` (#780).
///
/// Off the UI thread and shaped exactly like [`load_contacts`], for a
/// weaker version of the same reason: an account's labels are a handful
/// rather than a table, but the read still goes through the same pool every
/// pane is waiting on at startup, and there is no reason for the window to
/// hold still for it.
///
/// Read once, when the search surface is installed. A label created after
/// that is not offered until the next start -- which is the same limit the
/// correspondents list has, and worth stating rather than discovering: there
/// is no label-creation surface yet, so nothing can create one mid-session.
async fn load_labels(finder: &Finder, account: AccountId, client: &Client) {
    let client = client.clone();
    glib::spawn_future_local({
        let finder = finder.clone();
        async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
            // on its own runtime.
            let found = client.labels(account).await;
            let Ok(labels) =
                found.map_err(|error| tracing::warn!(%error, "could not read the labels"))
            else {
                return;
            };
            tracing::debug!(count = labels.len(), "labels read");
            finder.set_labels(&labels);
        }
    });
}

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
    async fn a_message_with_a_body(
        body: &str,
    ) -> (postio_storage::test_support::TempStore, AccountId) {
        let database = test_support::temp().await;
        let connection = database.connect().await.expect("checkout");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;

        let mut message = postio_model::Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some("Weekly notes".to_owned());
        message.sync.body_state = postio_model::BodyState::Full;
        let messages = MessageRepository::new(&connection);
        messages.create(&mut message).await.expect("create");

        messages
            .set_body(
                message.id,
                &postio_storage::repository::StoredBody {
                    text: Some(body.to_owned()),
                    html: None,
                    // A search excerpt writes text, never a block.
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("store the body");
        postio_index::index::index_body(&connection, message.id.get(), Some(body))
            .await
            .expect("index it");
        drop(connection);
        (database, account.id)
    }

    async fn search_for(
        database: &postio_storage::test_support::TempStore,
        account: AccountId,
        text: &str,
    ) -> SearchResults {
        let connection = database.connect().await.expect("checkout");
        let query = postio_search::parse(text, chrono::Utc::now().date_naive());
        postio_session::search::execute(
            &connection,
            AccountScope::Account(account),
            &query,
            Scope::AllMail,
            postio_search::ResultOrder::Relevance,
        )
        .await
        .expect("a search")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hit_gets_an_excerpt_cut_from_the_body_it_matched_in() {
        let body = "Dear Ada,\n\nThe difference engine's seventh column is \
                    finished and the drawings are with the printer.\n";
        let (database, account) = a_message_with_a_body(body).await;

        let results = search_for(&database, account, "printer").await;

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

    #[tokio::test(flavor = "multi_thread")]
    async fn what_is_marked_is_a_word_the_query_actually_matched() {
        // The criterion ADR 0017 named as the thing that would falsify the
        // contentless decision: a highlight regenerated from the blob that
        // points at different words than FTS5 scored is worse than none.
        //
        // `maildir` contains `mail`, and a highlighter that searched for
        // substrings would paint it — for a query that did not match this
        // message on that word at all, because FTS5 tokenizes `maildir` as
        // one token.
        let (database, account) = a_message_with_a_body("the maildir is rebuilt nightly").await;

        // Quoted, because the box answers a bare word that found nothing
        // with the word it begins (ADR 0037, amended) -- `mail` would come
        // back as `maildir`. Quotes ask for the word exactly.
        assert!(
            search_for(&database, account, "\"mail\"")
                .await
                .hits
                .is_empty(),
            "the query does not match, so there is nothing to highlight"
        );

        // And when the box does answer `mail` with `maildir`, what is marked
        // is the whole word that matched, not the letters that were typed.
        let rewritten = search_for(&database, account, "mail").await;
        assert_eq!(
            rewritten
                .instead
                .as_ref()
                .map(|instead| instead.term.as_str()),
            Some("maildir")
        );
        let marked = postio_search::highlight::from_snippet(&rewritten.hits[0].snippet);
        assert_eq!(
            marked
                .matches
                .iter()
                .map(|range| &marked.text[range.clone()])
                .collect::<Vec<_>>(),
            vec!["maildir"],
            "snippet: {:?}",
            rewritten.hits[0].snippet
        );

        let results = search_for(&database, account, "maildir").await;
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

    #[tokio::test(flavor = "multi_thread")]
    async fn a_query_with_nothing_to_point_at_leaves_the_snippet_alone() {
        // A structured-only query — `is:unread`, `in:archive` — has no term
        // to mark, and every hit's snippet stays empty exactly as it did when
        // SQLite was cutting them.
        let (database, account) = a_message_with_a_body("anything at all").await;

        let results = search_for(&database, account, "is:unread").await;

        assert_eq!(results.hits.len(), 1);
        assert!(results.hits[0].snippet.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_hit_whose_body_is_not_on_this_machine_gets_no_excerpt_rather_than_a_wrong_one() {
        // The message matched on its subject; its body is still on the
        // server. An excerpt cut from nothing would be an empty line that
        // looks like a body with no match in it.
        let database = test_support::temp().await;
        let connection = database.connect().await.expect("checkout");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("schema");
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some("The printer is fixed".to_owned());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("create");
        drop(connection);

        let results = search_for(&database, account.id, "printer").await;

        assert_eq!(results.hits.len(), 1, "the subject still matched");
        assert!(results.hits[0].snippet.is_empty());
    }

    // -- reindexing_covers (#981) -------------------------------------------

    #[tokio::test(flavor = "multi_thread")]
    async fn a_single_account_search_asks_only_about_itself() {
        let reindexing: Reindexing = Default::default();
        let watched = AccountId::new(1);
        let other = AccountId::new(2);
        reindexing.borrow_mut().insert(other);

        assert!(
            !reindexing_covers(&reindexing, AccountScope::Account(watched)),
            "the account this search ran under is not the one rebuilding"
        );

        reindexing.borrow_mut().insert(watched);
        assert!(
            reindexing_covers(&reindexing, AccountScope::Account(watched)),
            "now it is"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_unified_search_asks_whether_anything_is_rebuilding_at_all() {
        let reindexing: Reindexing = Default::default();
        assert!(
            !reindexing_covers(&reindexing, AccountScope::Unified),
            "nothing is rebuilding yet"
        );

        reindexing.borrow_mut().insert(AccountId::new(7));
        assert!(
            reindexing_covers(&reindexing, AccountScope::Unified),
            "a unified view covers every account, so one of them rebuilding \
             is enough to raise the caveat"
        );
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
    //! `postio_account::backend::MockBackend` (the seam CLAUDE.md names for
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

    use std::sync::Arc;
    use std::time::Duration;

    use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
    use postio_core::bridge::event_channel;

    use postio_runtime::Engine;
    use postio_runtime::engine::{EngineParts, NetworkSource, SystemClock};
    use postio_storage::repository::{
        AccountRepository, ListQuery, ListScope, MailboxRepository, MessageRepository,
    };
    use postio_storage::{BlobStore, Store, test_support};

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
    async fn engine_over(backend: Arc<MockBackend>) -> (Store, Engine, tempfile::TempDir) {
        let directory = tempfile::tempdir().expect("a database directory");
        // `Store::open`, with no size to give it: the engine keeps its own
        // pool, so there is nothing to size. The comment above about
        // exhaustion still holds -- the test creates its own, by holding
        // connections.
        let database = Store::open(directory.path().join("postio.db"), &test_support::key())
            .await
            .expect("a database");
        let account = {
            let connection = database.connect().await.expect("a connection");
            test_support::account(&connection).await
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
            tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
                postio_account::secret::MemorySecretStore::default(),
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
    async fn stored(database: &Store, path: &str) -> u32 {
        let Ok(connection) = database.connect().await else {
            return 0;
        };
        let Ok(accounts) = AccountRepository::new(&connection).list().await else {
            return 0;
        };
        let Some(account) = accounts.into_iter().next() else {
            return 0;
        };
        let Ok(mailboxes) = MailboxRepository::new(&connection)
            .list_for_account(account.id)
            .await
        else {
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
            .await
            .unwrap_or(0)
    }

    /// Waits for `condition`, or gives up and says what was true when it did.
    ///
    /// A liveness bound and nothing else, deliberately enormous for the
    /// reason `postio-runtime/tests/sync_wave.rs` sets out: a deadline small
    /// enough to be a performance budget is a flake waiting for a loaded
    /// machine.
    async fn until<F, Fut>(what: &str, mut condition: F)
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let waited = tokio::time::timeout(Duration::from_secs(180), async {
            while !condition().await {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(waited.is_ok(), "timed out waiting for {what}");
    }

    /// #672's guarantee, asserted against the mechanism that now carries it.
    ///
    /// # What this used to test, and why it could not stay
    ///
    /// It exhausted the connection pool. The engine's sync lanes held every
    /// connection, a background read queued for the next one, and the claim
    /// was that a reading-pane load asking *second* got the freed connection
    /// *first* -- because `Pool::get_interactive` outranked it.
    ///
    /// There is no pool to exhaust. The engine keeps its own, `connect` does
    /// not queue, and `interactive_is_waiting` has nothing to report. The
    /// setup cannot be built, so the assertion it fed cannot be made.
    ///
    /// # What is asserted instead
    ///
    /// The user-visible half, which is what #672 was ever about: mid-backfill,
    /// with the engine working through a large folder, a reading-pane body
    /// load **completes**. Not "completes quickly" -- a shared runner cannot
    /// defend a millisecond figure, which is why `bench.yml` times nothing --
    /// but completes at all, while the backfill is still going.
    ///
    /// # What is no longer covered
    ///
    /// Ordering. Nothing here proves an interactive read overtakes a
    /// background one, because connections are no longer the contended
    /// resource. The contended resource is the writer, and `WriteGate` is
    /// what arbitrates it -- see `r2_whether_a_long_write_blocks_a_short_one`
    /// in postio-storage, which is why the gate survived the engine change.
    /// A read does not take the gate at all, so a read cannot be starved by
    /// one. That is a weaker guarantee than #672 had and it is written down
    /// here rather than quietly lost.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reading_pane_body_load_does_not_wait_for_the_backfill() {
        let backend = Arc::new(
            MockBackend::builder()
                .mailbox(folder("INBOX", 3))
                .mailbox(folder(BULK, BULK_MESSAGES))
                .mailbox(folder("Archive", 0))
                .mailbox(folder("Sent", 0))
                .mailbox(folder("Drafts", 0))
                .mailbox(folder("Trash", 0))
                .mailbox(folder("Junk", 0))
                .build(),
        );
        let (database, engine, _directory) = engine_over(backend).await;

        until("INBOX to fully arrive", || async {
            stored(&database, "INBOX").await >= 3
        })
        .await;
        until("the bulk backfill to be under way", || async {
            stored(&database, BULK).await > 100
        })
        .await;

        // A message that is definitely local, asked for the way the reading
        // pane asks.
        let message = {
            let connection = database.connect().await.expect("a connection");
            let accounts = AccountRepository::new(&connection)
                .list()
                .await
                .expect("the account");
            let account = accounts.first().expect("an account").id;
            let inbox = MailboxRepository::new(&connection)
                .by_path(account, "INBOX")
                .await
                .expect("a read")
                .expect("INBOX exists");
            postio_storage::repository::MessageRepository::new(&connection)
                .page(&postio_storage::repository::ListQuery {
                    scope: postio_storage::repository::ListScope::Mailbox(inbox.id),
                    limit: 1,
                    after: None,
                })
                .await
                .expect("a page")
                .first()
                .expect("INBOX has arrived")
                .id
        };

        let runtime = tokio::runtime::Handle::current();
        let answer = crate::search::ask(&database, &runtime, move |connection| async move {
            postio_storage::repository::MessageRepository::new(&connection)
                .get(message)
                .await
                .ok()
                .flatten()
        });

        let result = tokio::time::timeout(Duration::from_secs(30), answer.recv())
            .await
            .expect("the reading pane's read never completed while the backfill ran")
            .expect("an answer");
        assert!(
            result.is_some(),
            "the reading pane did not find the message it asked for"
        );

        engine.stop();
    }
}
