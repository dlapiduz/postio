//! The Contacts screen, fed from the store (specs/005-contacts).
//!
//! `postio-gtk`'s pane reads nothing; it asks. This module answers on the
//! runtime -- the view's length, its pages, a filter, the person under the
//! cursor -- and carries out the two verbs that leave the screen: "show mail"
//! runs a `with:` search over every address the person owns, and "write to"
//! opens a draft to their preferred address. Both close the screen first, so
//! what they open is never hidden behind it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_gtk::window::Window;
use postio_model::{AccountId, ContactId, ContactView, Draft};
use postio_session::Wiring;
use postio_storage::repository::{ContactCursor, ContactRepository};

use crate::search::ask;

/// How many people a filter shows before "keep typing" (research R5): the
/// bound on what one keystroke materialises.
const FILTER_CAP: u32 = 500;

/// Rows per page, the list's.
const PAGE: u32 = postio_gtk::list::PAGE_SIZE;

/// Where pages already read ended, by offset, for the view they were read
/// in -- so the next page seeks past the last row rather than walking from
/// the top (the message list's marks, `postio-runtime`'s `read_page`).
#[derive(Default)]
struct Marks {
    view: Option<ContactView>,
    at: BTreeMap<u32, ContactCursor>,
}

impl Marks {
    /// The nearest boundary at or before `offset`, and how far past it.
    fn start(&mut self, view: ContactView, offset: u32) -> (Option<ContactCursor>, u32) {
        if self.view != Some(view) {
            self.view = Some(view);
            self.at.clear();
        }
        match self.at.range(..=offset).next_back() {
            Some((at, cursor)) => (Some(cursor.clone()), offset - at),
            None => (None, offset),
        }
    }
}

/// Wires the Contacts screen to the store. `account` is the one a new
/// message comes from, for "write to".
pub async fn install(window: &Window, wiring: &Wiring, account: AccountId) {
    let pane = window.contacts();
    let marks: Rc<RefCell<Marks>> = Rc::default();

    // The view's length, or a filter's rows, whenever either changes.
    pane.connect_query({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        let marks = marks.clone();
        move |view, filter| {
            marks.borrow_mut().at.clear();
            let filtered = !filter.trim().is_empty();
            let asked = (view, filter.clone());
            let answer = ask(&database, &runtime, move |connection| async move {
                let contacts = ContactRepository::new(&connection);
                let result = if filtered {
                    contacts
                        .filtered(view, &filter, FILTER_CAP)
                        .await
                        .map(Listed::Rows)
                } else {
                    contacts
                        .count(view)
                        .await
                        .map(|total| Listed::Count(u32::try_from(total).unwrap_or(u32::MAX)))
                };
                result
                    .map_err(|error| tracing::warn!(%error, "could not list contacts"))
                    .ok()
            });
            let pane = pane.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(listed)) = answer.recv().await else {
                    return;
                };
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                // An answer for a view or filter the user has since moved on
                // from is dropped rather than drawn over the current one.
                if (pane.view(), pane.filter_text()) != asked {
                    return;
                }
                match listed {
                    Listed::Count(total) => {
                        pane.reset(total);
                    }
                    Listed::Rows(rows) => pane.show_rows(rows),
                }
            });
        }
    });

    // A page the list asked for.
    pane.connect_page({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        let marks = marks.clone();
        move |view, generation, page| {
            let offset = page * PAGE;
            let (cursor, skip) = marks.borrow_mut().start(view, offset);
            let answer = ask(&database, &runtime, move |connection| async move {
                ContactRepository::new(&connection)
                    .page_from(view, cursor.as_ref(), skip, PAGE)
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not read a page of contacts"))
                    .ok()
            });
            let pane = pane.clone();
            let marks = marks.clone();
            glib::spawn_future_local(async move {
                let rows = answer.recv().await.ok().flatten();
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                match rows {
                    Some(rows) => {
                        if let Some(last) = rows.last()
                            && marks.borrow().view == Some(view)
                        {
                            marks
                                .borrow_mut()
                                .at
                                .insert(offset + rows.len() as u32, ContactCursor::after(last));
                        }
                        let total = pane.model().n_items();
                        pane.deliver(generation, page, rows, total);
                    }
                    None => pane.abandon(generation, page),
                }
            });
        }
    });

    // The person under the cursor, in detail.
    pane.connect_cursor({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |person| {
            let Some(person) = person else {
                return;
            };
            let answer = ask(&database, &runtime, move |connection| async move {
                ContactRepository::new(&connection)
                    .detail(person)
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not read a contact"))
                    .ok()
                    .flatten()
            });
            let pane = pane.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(detail)) = answer.recv().await else {
                    return;
                };
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                // Only if the cursor is still on them: a detail for someone the
                // keyboard has already left would describe the wrong row.
                if pane.cursor_person().map(|row| row.id) == Some(detail.person.id) {
                    pane.set_detail(Some(detail));
                }
            });
        }
    });

    // "Show mail": every address they own, in one `with:` (research R6).
    pane.connect_show_mail({
        let window = window.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |person| {
            let answer = addresses_of(&database, &runtime, person);
            let window = window.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(addresses)) = answer.recv().await else {
                    return;
                };
                let Some(window) = window.upgrade() else {
                    return;
                };
                if addresses.is_empty() {
                    return;
                }
                window.contacts().close();
                // A turn later, once closing has given the keyboard and its
                // context back. Opened in the same turn, the search worked but
                // `Esc` out of it afterwards left the keyboard in the search
                // context rather than on the list it came from --
                // `contacts_screen.rs` is what saw it.
                let query = postio_ui::contacts::show_mail_query(&addresses);
                glib::idle_add_local_once(move || window.run_search(&query));
            });
        }
    });

    // "Write to": a draft to their preferred address, with the screen closed
    // first so the composer is never behind it.
    pane.connect_compose({
        let window = window.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |person| {
            let answer = ask(&database, &runtime, move |connection| async move {
                ContactRepository::new(&connection)
                    .get(person)
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not read a contact"))
                    .ok()
                    .flatten()
            });
            let window = window.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(person)) = answer.recv().await else {
                    return;
                };
                let Some(window) = window.upgrade() else {
                    return;
                };
                let Some(preferred) = person.preferred_address() else {
                    return;
                };
                let name = [person.name.as_deref(), person.seen_name.as_deref()]
                    .into_iter()
                    .flatten()
                    .map(str::trim)
                    .find(|name| !name.is_empty())
                    .map(str::to_owned);
                let mut draft = Draft::new(account);
                draft.to = vec![postio_model::EmailAddress::new(
                    name,
                    preferred.address.address.clone(),
                )];
                window.contacts().close();
                window.composer().open(draft);
            });
        }
    });
}

/// What a query is answered with: a view's length, or a filter's rows whole.
enum Listed {
    Count(u32),
    Rows(Vec<postio_model::ContactListRow>),
}

/// Every address `person` owns, preferred first, on the runtime.
fn addresses_of(
    database: &postio_storage::Store,
    runtime: &tokio::runtime::Handle,
    person: ContactId,
) -> async_channel::Receiver<Option<Vec<String>>> {
    ask(database, runtime, move |connection| async move {
        ContactRepository::new(&connection)
            .get(person)
            .await
            .map_err(|error| tracing::warn!(%error, "could not read a contact"))
            .ok()
            .flatten()
            .map(|person| {
                person
                    .addresses
                    .iter()
                    .map(|owned| owned.address.address.clone())
                    .collect()
            })
    })
}
