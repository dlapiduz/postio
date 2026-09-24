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
use postio_core::{Command, ContactAddressAction};
use postio_gtk::window::Window;
use postio_model::{AccountId, ContactId, ContactView, Draft};
use postio_session::Wiring;
use postio_storage::repository::{ContactCursor, ContactGroupRepository, ContactRepository};

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

    install_edits(window, wiring);
}

/// `m`: what the marked people could be called, read from the store and
/// decided by `join_name_choices`; `+`: an address added, or -- when a live
/// person has it -- the question whether to move it (FR-012, FR-015).
fn install_edits(window: &Window, wiring: &Wiring) {
    let pane = window.contacts();
    pane.connect_join_asked({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |people| {
            let answer = ask(&database, &runtime, move |connection| async move {
                let contacts = ContactRepository::new(&connection);
                let mut found = Vec::with_capacity(people.len());
                for id in people {
                    match contacts.get(id).await {
                        Ok(Some(person)) => found.push(person),
                        Ok(None) => {}
                        Err(error) => {
                            tracing::warn!(%error, "could not read a contact to join");
                            return None;
                        }
                    }
                }
                Some(found)
            });
            let pane = pane.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(people)) = answer.recv().await else {
                    return;
                };
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                if people.len() < 2 {
                    pane.tell("Those people are no longer here to join");
                    return;
                }
                pane.show_join(postio_ui::contacts::join_name_choices(&people));
            });
        }
    });

    // The groups above the people, and a group's members when the keyboard
    // is on it (FR-040).
    pane.connect_groups_asked({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move || {
            let answer = ask(&database, &runtime, move |connection| async move {
                ContactGroupRepository::new(&connection)
                    .list()
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not list groups"))
                    .ok()
            });
            let pane = pane.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(groups)) = answer.recv().await else {
                    return;
                };
                if let Some(pane) = pane.upgrade() {
                    pane.show_groups(groups);
                }
            });
        }
    });
    pane.connect_group_rows({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |group| {
            let answer = ask(&database, &runtime, move |connection| async move {
                ContactRepository::new(&connection)
                    .group_rows(group, FILTER_CAP)
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not list a group"))
                    .ok()
            });
            let pane = pane.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(rows)) = answer.recv().await else {
                    return;
                };
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                // Only if the keyboard is still on that group.
                if pane.shown_group() == Some(group) {
                    pane.show_rows(rows);
                }
            });
        }
    });
    // `Return` on a group: its mail, from or to any member (FR-042).
    pane.connect_group_mail({
        let window = window.downgrade();
        move |name| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.contacts().close();
            let query = postio_ui::contacts::group_mail_query(&name);
            glib::idle_add_local_once(move || window.run_search(&query));
        }
    });

    // `v i`: a vCard file, read and parsed off the main thread, applied in
    // one transaction, and summed up in a line (FR-050..FR-054). The log
    // gets counts only: the file is somebody's address book (FR-061).
    pane.connect_import({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        let events = wiring.events.clone();
        move |path| {
            let answer = ask(&database, &runtime, move |connection| async move {
                Some(read_and_apply(&connection, &path).await)
            });
            let pane = pane.clone();
            let events = events.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(result)) = answer.recv().await else {
                    return;
                };
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                match result {
                    Ok(summary) => {
                        tracing::info!(
                            created = summary.people_created,
                            updated = summary.people_updated,
                            joined = summary.joins.len(),
                            groups = summary.groups_created,
                            name_conflicts = summary.name_conflicts,
                            skipped = summary.skipped.len(),
                            "imported contacts"
                        );
                        pane.tell(&postio_ui::contacts::import_summary_line(&summary));
                        events.emit(postio_core::Event::ContactsChanged {
                            names_changed: true,
                        });
                    }
                    Err(reason) => pane.tell(&reason),
                }
            });
        }
    });
    // `v x`: the marked people, or what the list shows, as one 4.0 file.
    pane.connect_export({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |scope, path| {
            let answer = ask(&database, &runtime, move |connection| async move {
                Some(read_and_write(&connection, scope, &path).await)
            });
            let pane = pane.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(result)) = answer.recv().await else {
                    return;
                };
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                match result {
                    Ok(count) => {
                        tracing::info!(count, "exported contacts");
                        pane.tell(&match count {
                            1 => "Exported 1 person".to_owned(),
                            n => format!("Exported {n} people"),
                        });
                    }
                    Err(reason) => pane.tell(&reason),
                }
            });
        }
    });

    // `v s`: who might be the same person, read whole and bounded (R11).
    pane.connect_suggestions_asked({
        let pane = pane.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move || {
            let answer = ask(&database, &runtime, move |connection| async move {
                ContactRepository::new(&connection)
                    .suggestions(SUGGESTIONS)
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not read suggestions"))
                    .ok()
            });
            let pane = pane.clone();
            glib::spawn_future_local(async move {
                let Ok(Some(suggestions)) = answer.recv().await else {
                    return;
                };
                if let Some(pane) = pane.upgrade() {
                    pane.show_suggestions(suggestions);
                }
            });
        }
    });

    pane.connect_add_address({
        let window = window.downgrade();
        let database = wiring.database.clone();
        let runtime = wiring.runtime.clone();
        move |person, text| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let parsed = postio_model::address::parse_list(&text);
            let [typed] = parsed.as_slice() else {
                window.contacts().tell("That is not one address");
                return;
            };
            if !typed.is_plausible() {
                window.contacts().tell("That does not look like an address");
                return;
            }
            let typed = postio_model::EmailAddress::new(None::<String>, typed.address.clone());
            let lookup = typed.address.clone();
            let answer = ask(&database, &runtime, move |connection| async move {
                ContactRepository::new(&connection)
                    .by_address(&lookup)
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not look an address up"))
                    .ok()
            });
            let window = window.downgrade();
            glib::spawn_future_local(async move {
                let Ok(Some(owner)) = answer.recv().await else {
                    return;
                };
                let Some(window) = window.upgrade() else {
                    return;
                };
                match owner {
                    // Someone the user can see has it: theirs to decide.
                    Some(owner)
                        if owner.id != person
                            && owner.state == postio_model::ContactState::Live =>
                    {
                        let normalized = typed.address.to_lowercase();
                        let Some(owned) = owner
                            .addresses
                            .iter()
                            .find(|a| a.address.address.to_lowercase() == normalized)
                        else {
                            return;
                        };
                        window.contacts().confirm_move(
                            owned.id,
                            &typed.address,
                            owner.display_name(),
                            person,
                        );
                    }
                    // Nobody, a deleted person (it moves, FR-024), or already
                    // theirs: the store answers.
                    _ => window.act(Command::ContactAddAddress(ContactAddressAction::Add {
                        person,
                        address: typed,
                    })),
                }
            });
        }
    });
}

/// Reads the file at `path`, parses it and applies it; the summary, or why
/// not in words for the screen.
async fn read_and_apply(
    connection: &postio_storage::Checkout,
    path: &std::path::Path,
) -> Result<postio_model::card::ImportSummary, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("Could not read that file: {error}"))?;
    let import = postio_vcard::parse(&bytes);
    let mut summary = ContactRepository::new(connection)
        .apply_import(&import.cards)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not apply an import");
            "Could not import those contacts".to_owned()
        })?;
    summary.skipped = import.skipped;
    Ok(summary)
}

/// Reads who `scope` names and writes them to `path`; how many, or why not.
async fn read_and_write(
    connection: &postio_storage::Checkout,
    scope: postio_gtk::contacts::ExportScope,
    path: &std::path::Path,
) -> Result<usize, String> {
    let contacts = ContactRepository::new(connection);
    let failed = |error: postio_storage::Error| {
        tracing::warn!(%error, "could not read contacts to export");
        "Could not export those contacts".to_owned()
    };
    let ids = match scope {
        postio_gtk::contacts::ExportScope::People(ids) => ids,
        postio_gtk::contacts::ExportScope::View(view) => {
            contacts.view_ids(view).await.map_err(failed)?
        }
    };
    let rows = contacts.export_people(&ids).await.map_err(failed)?;
    let groups = contacts.export_groups(&rows).await.map_err(failed)?;
    std::fs::write(path, export_file(&rows, &groups))
        .map_err(|error| format!("Could not write that file: {error}"))?;
    Ok(rows.len())
}

/// One `.vcf` of `rows`, each a 4.0 card edited into the one they were
/// imported from, when they were -- then the groups they are in, naming
/// only the members this file carries.
fn export_file(
    rows: &[postio_storage::repository::ExportRow],
    groups: &[postio_storage::repository::GroupExport],
) -> String {
    let people = rows
        .iter()
        .map(|row| {
            let person = &row.person;
            let emails: Vec<(String, bool)> = person
                .addresses
                .iter()
                .map(|owned| (owned.address.address.clone(), owned.id == person.preferred))
                .collect();
            let name = person.display_name();
            postio_vcard::export(
                &postio_vcard::ExportPerson {
                    uid: &row.uid,
                    name: (!name.is_empty()).then_some(name),
                    emails: &emails,
                    organization: person.organization.as_deref(),
                    note: person.note.as_deref(),
                },
                row.vcard.as_deref(),
            )
        })
        .collect::<String>();
    let groups = groups
        .iter()
        .map(|group| {
            postio_vcard::export_group(
                &group.uid,
                &group.name,
                &group.members,
                group.vcard.as_deref(),
            )
        })
        .collect::<String>();
    people + &groups
}

/// How many possible duplicates the view lists at once: a page of them to
/// work through, not the whole address book's worth.
const SUGGESTIONS: u32 = 50;

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
