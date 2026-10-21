//! vCard in and out of the address book (specs/005-contacts User Story 6).
//!
//! The bytes, not the files: a frontend reads the file the person chose and
//! writes the one they named, because a file is only ever read or written
//! through a panel they used (spec 011's contract), and this is the half
//! under it -- parse and apply, or read and write out. Neither touches the
//! network, and the log gets counts, never a name or an address: the file is
//! somebody's address book (FR-061).

use postio_model::card::ImportSummary;
use postio_model::{ContactId, ContactView};
use postio_storage::Store;
use postio_storage::repository::{ContactRepository, ExportRow, GroupExport};

/// Who an export writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportScope {
    /// These people, in this order: the marked ones, or what a filter shows.
    People(Vec<ContactId>),
    /// Everyone a view lists.
    View(ContactView),
}

/// Reads the cards in `bytes` and applies them in one transaction. The
/// summary, or why not in words for the screen.
pub async fn import_vcards(database: &Store, bytes: &[u8]) -> Result<ImportSummary, String> {
    let import = postio_vcard::parse(bytes);
    let connection = database.connect().await.map_err(|error| {
        tracing::warn!(%error, "could not open the store to import");
        "Could not import those contacts".to_owned()
    })?;
    let mut summary = ContactRepository::new(&connection)
        .apply_import(&import.cards)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not apply an import");
            "Could not import those contacts".to_owned()
        })?;
    summary.skipped = import.skipped;
    tracing::info!(
        created = summary.people_created,
        updated = summary.people_updated,
        offered_merges = summary.offered_merges,
        look_alikes = summary.look_alikes,
        groups = summary.groups_created,
        name_conflicts = summary.name_conflicts,
        skipped = summary.skipped.len(),
        "imported contacts"
    );
    Ok(summary)
}

/// Reads who `scope` names and returns one `.vcf` of them, and how many
/// people it holds.
pub async fn export_vcards(
    database: &Store,
    scope: ExportScope,
) -> Result<(String, usize), String> {
    let failed = |error: postio_storage::Error| {
        tracing::warn!(%error, "could not read contacts to export");
        "Could not export those contacts".to_owned()
    };
    let connection = database.read().await.map_err(failed)?;
    let contacts = ContactRepository::new(&connection);
    let ids = match scope {
        ExportScope::People(ids) => ids,
        ExportScope::View(view) => contacts.view_ids(view).await.map_err(failed)?,
    };
    let rows = contacts.export_people(&ids).await.map_err(failed)?;
    let groups = contacts.export_groups(&rows).await.map_err(failed)?;
    tracing::info!(count = rows.len(), "exported contacts");
    Ok((file_of(&rows, &groups), rows.len()))
}

/// One `.vcf` of `rows`, each a 4.0 card edited into the one they were
/// imported from, when they were -- then the groups they are in, naming only
/// the members this file carries.
fn file_of(rows: &[ExportRow], groups: &[GroupExport]) -> String {
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

/// The person at `address`, made now from the stored mail when the catch-up
/// has not reached them yet; `None` for an address the store never saw and
/// for the user's own.
///
/// What a card or `i` on a sender runs, so nothing waits on the catch-up.
pub async fn contact_for_address(database: &Store, address: &str) -> Option<postio_model::Contact> {
    let connection = database.connect().await.ok()?;
    let contacts = ContactRepository::new(&connection);
    if let Ok(Some(person)) = contacts.by_address(address).await {
        return Some(person);
    }
    let id = postio_storage::sql::first(
        &connection,
        "SELECT id FROM addresses WHERE address_normalized = ?1",
        postio_storage::sql::bind![address.trim().to_lowercase()],
        |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
    )
    .await
    .ok()??;
    let person = contacts
        .ensure_for_address(postio_model::AddressId::new(id))
        .await
        .ok()?;
    contacts.get(person).await.ok()?
}

/// What the known-sender popover shows for `address`: the person (made now if
/// the catch-up has not reached them), their counts, up to two things open
/// between them and the user, and where their mail arrives. `None` for an
/// address the store never saw.
///
/// One read over maintained tables: it costs the person's addresses, never
/// their mail.
pub async fn contact_glance(
    database: &Store,
    address: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<postio_model::ContactGlance> {
    use postio_storage::repository::ContactSettingsRepository;
    let person = contact_for_address(database, address).await?;
    let reader = database.read().await.ok()?;
    let contacts = ContactRepository::new(&reader);
    let stats = contacts
        .stats(person.id, postio_model::month_key(now))
        .await
        .ok()?;
    let open = contacts
        .open_items(person.id, postio_model::contact::GLANCE_OPEN)
        .await
        .ok()?;
    let settings = ContactSettingsRepository::new(&reader)
        .get(person.id)
        .await
        .ok()?;
    let default_arrival = contacts.default_arrival(person.id).await.ok()?;
    // Someone only the mail knows gets introduced, unless the user said
    // never to.
    let introduction = if !person.is_saved()
        && person.source == postio_model::ContactSource::Mail
        && person.kind == postio_model::ContactKind::Person
        && !settings.ignored
    {
        introduction(database, &reader, &person).await
    } else {
        None
    };
    Some(postio_model::ContactGlance {
        person,
        stats,
        open,
        arrival: settings.arrival,
        default_arrival,
        introduction,
    })
}

/// What the newest mail from `person` says that the Edit form can keep: the
/// phones and websites in its signature that are neither kept nor refused,
/// then the ones kept. Empty when there is nothing, which is also what a
/// failed read says: the form simply has no "Seen in mail".
pub async fn contact_seen(
    database: &Store,
    person: postio_model::ContactId,
) -> Vec<postio_model::SeenItem> {
    use postio_model::{DetailKind, SeenItem};
    let Ok(reader) = database.read().await else {
        return Vec::new();
    };
    let contacts = ContactRepository::new(&reader);
    let Ok(Some(held)) = contacts.get(person).await else {
        return Vec::new();
    };
    let known = contacts.details(person).await.unwrap_or_default();
    let mut items = Vec::new();
    if let Ok(Some((message, _, _))) = contacts.introduction_facts(&held).await {
        let body = crate::search::stored_body(database, message).await;
        let parsed = postio_classify::signature::parse(body.text.as_deref().unwrap_or(""));
        for (kind, value) in parsed
            .phones
            .into_iter()
            .map(|v| (DetailKind::Phone, v))
            .chain(parsed.websites.into_iter().map(|v| (DetailKind::Url, v)))
        {
            if !known.iter().any(|d| d.kind == kind && d.value == value) {
                items.push(SeenItem {
                    kind,
                    value,
                    kept: false,
                });
            }
        }
    }
    items.extend(known.into_iter().filter(|d| d.added).map(|d| SeenItem {
        kind: d.kind,
        value: d.value,
        kept: true,
    }));
    items
}

/// What the review offers to merge: the pairs the store suggests, scored with
/// the evidence a person can read, joined into groups of people who look like
/// one, the strongest first (specs/011-focus-contacts S10). A pair with no
/// evidence is no candidate.
pub async fn contact_review(database: &Store) -> Vec<postio_model::MergeGroup> {
    let Ok(reader) = database.read().await else {
        return Vec::new();
    };
    let contacts = ContactRepository::new(&reader);
    let Ok(mut pairs) = contacts.suggestion_pairs(40).await else {
        return Vec::new();
    };
    // And the pairs nobody suggested: what the mail noted (a signature, a
    // phone, a Resent-From) and one address stopping as another began.
    for (a, b) in contacts.discovered_pairs(40).await.unwrap_or_default() {
        if pairs
            .iter()
            .any(|(x, y, _)| (x.id, y.id) == (a, b) || (x.id, y.id) == (b, a))
        {
            continue;
        }
        if let (Ok(Some(a)), Ok(Some(b))) = (contacts.get(a).await, contacts.get(b).await) {
            pairs.push((a, b, 0));
        }
    }
    let mut scored = Vec::new();
    for (a, b, replies) in pairs {
        if let Some((evidence, score)) = evidence_of(database, &contacts, &a, &b, replies).await {
            scored.push((a, b, evidence, score));
        }
    }
    groups_of(scored)
}

/// The group the user chose by marking `ids`: whatever evidence there is is
/// said, and when there is none the user's own choice is.
pub async fn contact_merge_of(
    database: &Store,
    ids: &[postio_model::ContactId],
) -> Option<postio_model::MergeGroup> {
    let reader = database.read().await.ok()?;
    let contacts = ContactRepository::new(&reader);
    let mut people = Vec::new();
    for id in ids {
        people.push(contacts.get(*id).await.ok()??);
    }
    if people.len() < 2 {
        return None;
    }
    let mut lists = Vec::new();
    for (i, a) in people.iter().enumerate() {
        for b in &people[i + 1..] {
            if let Some((evidence, _)) = evidence_of(database, &contacts, a, b, 0).await {
                lists.push(evidence);
            }
        }
    }
    people.sort_by_key(|p| {
        (
            std::cmp::Reverse(p.is_saved()),
            std::cmp::Reverse(p.times_seen),
        )
    });
    let mut lines =
        postio_classify::duplicates::describe(people.len(), &lists, people[0].shown_as().as_ref());
    if lines.is_empty() {
        lines.push("You chose these records.".to_owned());
    }
    Some(postio_model::MergeGroup {
        people,
        evidence: lines,
    })
}

/// What two records have in common that says they may be one person, and how
/// much it counts for.
async fn evidence_of(
    database: &Store,
    contacts: &ContactRepository<'_>,
    a: &postio_model::Contact,
    b: &postio_model::Contact,
    replies: u32,
) -> Option<(Vec<postio_classify::duplicates::Evidence>, u32)> {
    use postio_classify::duplicates::{Pair, Side, score};
    let side = |person: &postio_model::Contact, signature: &postio_model::SignatureDetails| Side {
        name: person.shown_as().into_owned(),
        addresses: person
            .addresses
            .iter()
            .map(|owned| owned.address.address.clone())
            .collect(),
        title: person.title.clone().or_else(|| signature.title.clone()),
        company: person
            .organization
            .clone()
            .or_else(|| signature.company.clone()),
    };
    let (sig_a, sig_b) = (
        signature_of(database, contacts, a).await,
        signature_of(database, contacts, b).await,
    );
    let took = contacts.took_over(a, b).await.ok().flatten();
    let noted = contacts.pair_evidence(a, b).await.unwrap_or_default();
    let pair = Pair {
        took_over_in_threads: took.as_ref().map_or(0, |(_, _, n)| *n),
        took_over: took.map(|(old, new, _)| (old, new)),
        replies,
        shared_phone: noted.phone,
        same_card: noted.card,
        signature: noted.signature.and_then(|key| {
            let mut parts = key.split('|').skip(1);
            // The spelling a record already has, else the folded words said
            // the way a person would write them.
            let spelled = |folded: &str, real: [&Option<String>; 2]| {
                real.into_iter()
                    .flatten()
                    .find(|text| text.to_lowercase() == folded)
                    .cloned()
                    .unwrap_or_else(|| words_cased(folded))
            };
            Some((
                spelled(parts.next()?, [&a.title, &b.title]),
                spelled(parts.next()?, [&a.organization, &b.organization]),
            ))
        }),
        forwards_from: noted.resent.as_ref().map(|(from, _)| from.clone()),
        forwards_to: noted.resent.map(|(_, to)| to),
    };
    let scored = score(&side(a, &sig_a), &side(b, &sig_b), &pair)?;
    Some((scored.evidence, scored.score))
}

/// "finance lead" as "Finance Lead": what the key keeps folded, said the way a
/// person would write it when no record has the real spelling.
fn words_cased(folded: &str) -> String {
    folded
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What the newest mail from `person` signs with.
async fn signature_of(
    database: &Store,
    contacts: &ContactRepository<'_>,
    person: &postio_model::Contact,
) -> postio_model::SignatureDetails {
    let Ok(Some((message, _, _))) = contacts.introduction_facts(person).await else {
        return postio_model::SignatureDetails::default();
    };
    let body = crate::search::stored_body(database, message).await;
    postio_classify::signature::parse(body.text.as_deref().unwrap_or(""))
}

/// Joins scored pairs that share a person into groups: the best record to
/// keep first (saved, then the most mail), and the group's evidence in words.
fn groups_of(
    scored: Vec<(
        postio_model::Contact,
        postio_model::Contact,
        Vec<postio_classify::duplicates::Evidence>,
        u32,
    )>,
) -> Vec<postio_model::MergeGroup> {
    let mut groups: Vec<(
        Vec<postio_model::Contact>,
        Vec<Vec<postio_classify::duplicates::Evidence>>,
        u32,
    )> = Vec::new();
    for (a, b, evidence, score) in scored {
        let at = groups
            .iter()
            .position(|(people, _, _)| people.iter().any(|p| p.id == a.id || p.id == b.id));
        match at {
            Some(at) => {
                let (people, lists, total) = &mut groups[at];
                for person in [a, b] {
                    if !people.iter().any(|p| p.id == person.id) {
                        people.push(person);
                    }
                }
                lists.push(evidence);
                *total = (*total).max(score);
            }
            None => groups.push((vec![a, b], vec![evidence], score)),
        }
    }
    // The strongest evidence first, and among equals the most records.
    groups.sort_by_key(|(people, _, score)| std::cmp::Reverse((*score, people.len())));
    groups
        .into_iter()
        .map(|(mut people, lists, _)| {
            people.sort_by_key(|p| {
                (
                    std::cmp::Reverse(p.is_saved()),
                    std::cmp::Reverse(p.times_seen),
                )
            });
            let evidence = postio_classify::duplicates::describe(
                people.len(),
                &lists,
                people[0].shown_as().as_ref(),
            );
            postio_model::MergeGroup { people, evidence }
        })
        .collect()
}

/// How a sender not yet saved reached the user: when their newest message
/// came, who else was on it that the user knows, and what its signature says.
///
/// The body is read from the store, never fetched, and parsed on this
/// machine; what it said is kept nowhere until the user adds the contact.
async fn introduction(
    database: &Store,
    reader: &postio_storage::Checkout,
    person: &postio_model::Contact,
) -> Option<postio_model::Introduction> {
    let (message, received, with) = ContactRepository::new(reader)
        .introduction_facts(person)
        .await
        .ok()
        .flatten()?;
    let body = crate::search::stored_body(database, message).await;
    let details = postio_classify::signature::parse(body.text.as_deref().unwrap_or(""));
    Some(postio_model::Introduction {
        details,
        last_at: Some(received),
        with,
    })
}

/// How many rows a filtered Contacts list holds at most.
pub const FILTER_CAP: u32 = 200;

fn store_error(error: postio_storage::Error) -> postio_model::listing::StoreError {
    tracing::warn!(%error, "could not read contacts");
    postio_model::listing::StoreError::new("Could not read contacts")
}

/// A page of the Contacts list for `tab`: past `after`, then `skip` more.
/// The page carries the sort key of the row before it, so the list can tell
/// whether its first row starts a new letter without a read of its own.
pub async fn contacts_page(
    database: &Store,
    tab: postio_model::ContactTab,
    after: Option<(String, postio_model::ContactId)>,
    skip: u32,
    limit: u32,
) -> Result<postio_model::ContactsPageRead, postio_model::listing::StoreError> {
    let reader = database.read().await?;
    let contacts = ContactRepository::new(&reader);
    let cursor = after
        .as_ref()
        .map(|(key, id)| postio_storage::repository::ContactCursor::from_parts(key.clone(), *id));
    if skip == 0 {
        let rows = contacts
            .page_kinds(tab.kinds(), cursor.as_ref(), 0, limit)
            .await
            .map_err(store_error)?;
        return Ok(postio_model::ContactsPageRead {
            rows,
            before: after.map(|(key, _)| key),
        });
    }
    // One row early, to know what the first row follows.
    let mut rows = contacts
        .page_kinds(tab.kinds(), cursor.as_ref(), skip - 1, limit + 1)
        .await
        .map_err(store_error)?;
    let before = (!rows.is_empty()).then(|| rows.remove(0).sort_key);
    Ok(postio_model::ContactsPageRead { rows, before })
}

/// The people in `tab` matching `text`, at most `cap`.
pub async fn contacts_filtered(
    database: &Store,
    tab: postio_model::ContactTab,
    text: &str,
    cap: u32,
) -> Result<Vec<postio_model::ContactListRow>, postio_model::listing::StoreError> {
    let reader = database.read().await?;
    ContactRepository::new(&reader)
        .filtered_kinds(tab.kinds(), text, cap.min(FILTER_CAP))
        .await
        .map_err(store_error)
}

/// The strip's counts, the review queue and the catch-up's progress.
pub async fn contacts_overview(
    database: &Store,
) -> Result<postio_model::ContactsOverview, postio_model::listing::StoreError> {
    let reader = database.read().await?;
    ContactRepository::new(&reader)
        .overview()
        .await
        .map_err(store_error)
}

/// What `person`'s card shows, as of `now`: the twelve months ending this
/// month.
pub async fn contact_card(
    database: &Store,
    person: postio_model::ContactId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<postio_model::ContactCardRead>, postio_model::listing::StoreError> {
    let reader = database.read().await?;
    ContactRepository::new(&reader)
        .card(person, postio_model::month_key(now) - 11)
        .await
        .map_err(store_error)
}

/// The name of every contact group, by name: what "+ Add to group" offers.
pub async fn contact_group_names(
    database: &Store,
) -> Result<Vec<String>, postio_model::listing::StoreError> {
    let reader = database.read().await?;
    postio_storage::repository::ContactGroupRepository::new(&reader)
        .list()
        .await
        .map(|groups| groups.into_iter().map(|group| group.name).collect())
        .map_err(store_error)
}

/// How far the catch-up has got: "Building contacts, n of m".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchUpProgress {
    /// People made so far this run.
    pub made: usize,
    /// How many addresses were waiting when it began.
    pub total: usize,
}

/// Makes a person for every address that has mail and none, `batch` at a
/// time, newest first, one transaction per batch so live sync is never held
/// up, and tells `progress` after each. Returns how many it made. The log
/// gets counts; a name or an address never reaches it.
pub async fn catch_up(
    database: &Store,
    batch: u32,
    mut progress: impl FnMut(CatchUpProgress),
) -> Result<usize, String> {
    let failed = |error: postio_storage::Error| {
        tracing::warn!(%error, "could not build contacts");
        "Could not build contacts".to_owned()
    };
    let connection = database.connect().await.map_err(failed)?;
    let contacts = ContactRepository::new(&connection);
    let total = contacts.unowned_count().await.map_err(failed)? as usize;
    let (mut made, mut before) = (0, None);
    loop {
        let step = contacts
            .catch_up_batch(before, batch)
            .await
            .map_err(failed)?;
        made += step.made;
        before = step.last;
        if step.made > 0 {
            progress(CatchUpProgress { made, total });
        }
        if step.done {
            break;
        }
    }
    if made > 0 {
        tracing::info!(made, total, "built contacts");
    }
    Ok(made)
}

/// Decides the kind of everyone the current detector has not decided yet,
/// `batch` at a time, and stores it. Returns how many it decided. A kind the
/// user chose is never touched, and a better detector (a higher
/// `KIND_VERSION`) decides everyone detected again.
///
/// Reads counts over each sender's newest messages (storage's
/// `kind_signals`), never mail, and logs counts only.
pub async fn detect_kinds(database: &Store, batch: u32) -> Result<usize, String> {
    let failed = |error: postio_storage::Error| {
        tracing::warn!(%error, "could not decide what sort of correspondents people are");
        "Could not sort contacts".to_owned()
    };
    let connection = database.connect().await.map_err(failed)?;
    let contacts = ContactRepository::new(&connection);
    let rules = postio_classify::kind::KindRules::shipped();
    let version = postio_classify::kind::KIND_VERSION;
    let mut decided = 0;
    loop {
        let people = contacts
            .pending_kinds(version, batch)
            .await
            .map_err(failed)?;
        for person in &people {
            // Someone with no mail to count (no preferred address) is a
            // person until there is some; deciding them anyway is what takes
            // them out of the walk.
            let kind = match contacts.kind_signals(*person).await.map_err(failed)? {
                Some(signals) => postio_classify::kind::detect(&signals, rules).kind,
                None => postio_model::ContactKind::Person,
            };
            contacts
                .set_detected_kind(*person, kind, version)
                .await
                .map_err(failed)?;
            decided += 1;
        }
        if people.len() < batch as usize {
            break;
        }
    }
    if decided > 0 {
        tracing::info!(decided, version, "sorted contacts by kind");
    }
    Ok(decided)
}

/// Every sender's remembered treatment, `(address, "app" | "paper")`: one
/// entry per address of each person who chose one.
pub async fn treatments(
    database: &Store,
) -> Result<Vec<(String, String)>, postio_model::listing::StoreError> {
    let reader = database.read().await?;
    Ok(
        postio_storage::repository::ContactSettingsRepository::new(&reader)
            .treatments()
            .await?,
    )
}

/// Remember that `address`'s mail is always drawn in `treatment`, or forget
/// the choice. The treatment is the person's, so it covers every address they
/// own; one who has never been seen is made a person to hold it.
pub async fn set_treatment(
    database: &Store,
    address: &str,
    treatment: Option<&str>,
) -> Result<(), postio_model::listing::StoreError> {
    use postio_storage::repository::ContactSettingsRepository;
    if treatment.is_some_and(|t| !matches!(t, "app" | "paper")) {
        return Err(postio_model::listing::StoreError::new(
            "A treatment is app or paper",
        ));
    }
    let connection = database.connect().await?;
    let people = ContactRepository::new(&connection);
    let email = postio_model::EmailAddress::new(None::<String>, address.trim());
    let person = match (people.by_address(address.trim()).await?, treatment) {
        (Some(person), _) => person.id,
        // Forgetting a choice nobody made makes nobody.
        (None, None) => return Ok(()),
        (None, Some(_)) => people
            .person_for_email(&email)
            .await
            .map_err(|_| postio_model::listing::StoreError::new("That is not a sender"))?,
    };
    let settings = ContactSettingsRepository::new(&connection);
    let mut held = settings.get(person).await?;
    held.html_treatment = treatment.map(str::to_owned);
    settings.put(person, &held).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use postio_storage::test_support;

    use super::*;

    const FILE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Yara Quux\r\n\
EMAIL;TYPE=INTERNET,PREF:yara@home.example\r\nEMAIL:yara@work.example\r\n\
X-VENDOR:kept\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:z\r\nFN:Zebulon Quux\r\nEMAIL:zq@example.net\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nKIND:group\r\nFN:Quuxes\r\nMEMBER:urn:uuid:z\r\n\
MEMBER:mailto:yara@work.example\r\nEND:VCARD\r\n";

    /// Everything logged on this thread while it is installed.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log buffer").extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    async fn egress_rows(database: &Store) -> i64 {
        let connection = database.connect().await.expect("a connection");
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM egress_log", ())
            .await
            .expect("count the egress log")
    }

    /// The Mac's `v i` and `v x` end here: the people and the group appear,
    /// a second export of what the list shows is exactly those people, the
    /// card's own lines survive, nothing leaves the machine, and the log
    /// carries counts and never a name.
    #[tokio::test]
    async fn an_import_then_an_export_round_trips_and_leaks_nothing() {
        let log = Captured::default();
        let _logging = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_ansi(false)
                .with_writer({
                    let log = log.clone();
                    move || log.clone()
                })
                .finish(),
        );
        let database = test_support::memory().await;
        postio_storage::seed::seed_small(&database, 11).await;
        let egress_before = egress_rows(&database).await;

        let summary = import_vcards(&database, FILE.as_bytes())
            .await
            .expect("the import");
        assert_eq!(summary.people_created, 2);
        assert_eq!(summary.groups_created, 1);
        let line = postio_ui::contacts::import_summary_line(&summary);
        assert!(
            line.contains("2 new people") && line.contains("1 group"),
            "{line}"
        );

        let (written, count) = export_vcards(&database, ExportScope::View(ContactView::Written))
            .await
            .expect("the export");
        assert!(written.contains("FN:Yara Quux") && written.contains("FN:Zebulon Quux"));
        assert!(
            written.contains("X-VENDOR:kept"),
            "the card's own lines survive"
        );
        assert!(written.contains("FN:Quuxes"), "and the group they are in");
        let people = written.matches("BEGIN:VCARD").count() - written.matches("KIND:group").count();
        assert_eq!(
            people, count,
            "exactly the people the view lists -- none of the mail-only senders"
        );

        assert_eq!(egress_rows(&database).await, egress_before);
        let logged = String::from_utf8_lossy(&log.0.lock().expect("log")).into_owned();
        assert!(logged.contains("imported contacts"), "{logged}");
        for private in ["Yara", "Zebulon", "yara@", "zq@", "Quuxes"] {
            assert!(
                !logged.contains(private),
                "the log named {private}: {logged}"
            );
        }
    }

    /// Mail that was stored before contacts existed: a message from `from`
    /// to the account's own address, with nobody made of either.
    async fn old_mail(database: &Store, from: &str, days: i64) {
        use chrono::TimeZone as _;
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(
            account.id,
            inbox,
            chrono::Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap()
                + chrono::Duration::days(days),
        );
        message.from = vec![postio_model::EmailAddress::new(Some("Old Friend"), from)];
        message.to = vec![account.address.clone()];
        postio_storage::repository::MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("store old mail");
        test_support::mark_contacts_history(&connection).await;
    }

    #[tokio::test]
    async fn a_card_on_a_sender_with_no_person_makes_one_and_a_stranger_gets_none() {
        let database = test_support::memory().await;
        old_mail(&database, "friend@example.org", 1).await;

        let none = contact_for_address(&database, "nobody@example.org").await;
        assert!(
            none.is_none(),
            "an address the store never saw is not a person"
        );

        let made = contact_for_address(&database, "Friend@Example.org")
            .await
            .expect("a person is made on demand");
        assert_eq!(made.display_name(), "Old Friend");
        let again = contact_for_address(&database, "friend@example.org")
            .await
            .expect("and found again");
        assert_eq!(again.id, made.id, "one person, however often it is asked");
    }

    #[tokio::test]
    async fn a_glance_at_a_sender_is_the_person_their_counts_and_where_mail_arrives() {
        let database = test_support::memory().await;
        old_mail(&database, "friend@example.org", 1).await;
        use chrono::TimeZone as _;
        let now = chrono::Utc.with_ymd_and_hms(2026, 3, 9, 12, 0, 0).unwrap();

        assert!(
            contact_glance(&database, "nobody@example.org", now)
                .await
                .is_none()
        );
        let glance = contact_glance(&database, "Friend@Example.org", now)
            .await
            .expect("a glance at someone the mail knows");
        assert_eq!(glance.person.display_name(), "Old Friend");
        assert_eq!(glance.stats.messages, 1);
        assert_eq!(glance.arrival, None);
        assert_eq!(glance.default_arrival, postio_model::ContactArrival::Inbox);
        assert!(glance.open.is_empty());

        let connection = database.connect().await.unwrap();
        postio_storage::repository::ContactSettingsRepository::new(&connection)
            .put(
                glance.person.id,
                &postio_model::ContactSettings {
                    arrival: Some(postio_model::ContactArrival::Digest),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let again = contact_glance(&database, "friend@example.org", now)
            .await
            .unwrap();
        assert_eq!(again.arrival, Some(postio_model::ContactArrival::Digest));
    }

    /// A first message from `from`, copied to someone the user saved, with
    /// `body` stored beside it.
    async fn first_mail(database: &Store, from: &str, body: &str) {
        use chrono::TimeZone as _;
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let ben = postio_model::EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net");
        ContactRepository::new(&connection)
            .create(Some("Ben Adeyemi"), std::slice::from_ref(&ben))
            .await
            .expect("a saved contact");
        let mut message = postio_model::Message::new(
            account.id,
            inbox,
            chrono::Utc.with_ymd_and_hms(2026, 3, 9, 9, 0, 0).unwrap(),
        );
        message.from = vec![postio_model::EmailAddress::new(Some("Jonas Weber"), from)];
        message.to = vec![account.address.clone()];
        message.cc = vec![ben];
        let repository = postio_storage::repository::MessageRepository::new(&connection);
        repository.create(&mut message).await.expect("store mail");
        repository
            .set_body(
                message.id,
                &postio_storage::repository::StoredBody {
                    text: Some(body.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("a body");
        test_support::mark_contacts_history(&connection).await;
    }

    #[tokio::test]
    async fn a_sender_not_yet_saved_is_introduced_by_their_signature_and_who_was_on_copy() {
        use chrono::TimeZone as _;
        let database = test_support::memory().await;
        first_mail(
            &database,
            "jonas@harbordata.example",
            "Hi, I help teams move analytics off the warehouse.\n\nBest,\nJonas Weber\nPartnerships \u{b7} Harbor Data Co.\n+1 555 0142\nharbordata.example\n",
        )
        .await;
        let now = chrono::Utc.with_ymd_and_hms(2026, 3, 9, 12, 0, 0).unwrap();
        let glance = contact_glance(&database, "jonas@harbordata.example", now)
            .await
            .expect("a glance");
        let intro = glance.introduction.expect("not saved, so introduced");
        assert_eq!(intro.details.title.as_deref(), Some("Partnerships"));
        assert_eq!(intro.details.company.as_deref(), Some("Harbor Data Co."));
        assert_eq!(intro.details.phones, ["+1 555 0142"]);
        assert_eq!(intro.with, ["Ben Adeyemi"]);
        assert!(intro.last_at.is_some());

        // Someone saved is a known sender; so is someone told to be ignored.
        let ben = contact_glance(&database, "ben@example.net", now)
            .await
            .expect("ben");
        assert!(
            ben.introduction.is_none(),
            "saved people are not introduced"
        );

        let connection = database.connect().await.unwrap();
        postio_storage::repository::ContactSettingsRepository::new(&connection)
            .put(
                glance.person.id,
                &postio_model::ContactSettings {
                    ignored: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let again = contact_glance(&database, "jonas@harbordata.example", now)
            .await
            .unwrap();
        assert!(again.introduction.is_none(), "ignored, so never offered");
    }

    #[tokio::test]
    async fn the_demos_new_sender_is_introduced_the_way_screen_05_draws_it() {
        use chrono::TimeZone as _;
        let (database, _) = postio_demo::demo().await;
        let now = chrono::Utc.with_ymd_and_hms(2026, 3, 9, 12, 0, 0).unwrap();
        let glance = contact_glance(&database, postio_demo::NEW_SENDER, now)
            .await
            .expect("the demo's new sender");
        let intro = glance.introduction.expect("not saved, so introduced");
        assert_eq!(intro.details.title.as_deref(), Some("Partnerships"));
        assert_eq!(intro.details.company.as_deref(), Some("Harbor Data Co."));
        assert_eq!(intro.with, ["Ben Adeyemi"]);
    }

    #[tokio::test]
    async fn the_review_offers_look_alikes_with_their_evidence_and_nothing_without_any() {
        let database = test_support::memory().await;
        let (ada, a_moreno, ben) = {
            let connection = database.connect().await.unwrap();
            let contacts = ContactRepository::new(&connection);
            let make = |name: &'static str, address: &'static str| {
                let contacts = &contacts;
                async move {
                    contacts
                        .create(
                            Some(name),
                            &[postio_model::EmailAddress::new(None::<String>, address)],
                        )
                        .await
                        .unwrap()
                }
            };
            (
                make("Ada Moreno", "ada@example.org").await,
                make("ada  moreno", "a.moreno@example.net").await,
                make("Ben Adeyemi", "ben@example.net").await,
            )
        };
        let groups = contact_review(&database).await;
        assert_eq!(groups.len(), 1, "Ben matches nobody: {groups:?}");
        let ids: Vec<_> = groups[0].people.iter().map(|p| p.id).collect();
        assert!(ids.contains(&ada) && ids.contains(&a_moreno) && !ids.contains(&ben));
        assert_eq!(groups[0].evidence.len(), 1);
        assert!(
            groups[0].evidence[0].contains("Ada Moreno"),
            "{:?}",
            groups[0].evidence
        );

        // Chosen by hand, with nothing in common, the choice is the evidence.
        let chosen = contact_merge_of(&database, &[ada, ben])
            .await
            .expect("a group");
        assert_eq!(chosen.evidence, ["You chose these records."]);
        assert!(
            contact_merge_of(&database, &[ada]).await.is_none(),
            "one is not a merge"
        );
    }

    /// Two messages, one from each of two addresses of one person nobody
    /// told the store about, and the people the catch-up made of them.
    async fn two_addresses_one_person(
        database: &Store,
    ) -> (postio_model::MessageId, postio_model::MessageId) {
        use chrono::TimeZone as _;
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = postio_storage::repository::MessageRepository::new(&connection);
        let mut ids = Vec::new();
        for (from, day) in [("a.moreno@example.net", 1), ("ada@example.org", 40)] {
            let mut message = postio_model::Message::new(
                account.id,
                inbox,
                chrono::Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
                    + chrono::Duration::days(day),
            );
            message.from = vec![postio_model::EmailAddress::new(None::<String>, from)];
            message.to = vec![account.address.clone()];
            repository.create(&mut message).await.expect("a message");
            ids.push(message.id);
        }
        test_support::mark_contacts_history(&connection).await;
        ContactRepository::new(&connection)
            .catch_up_batch(None, 100)
            .await
            .expect("people");
        (ids[0], ids[1])
    }

    #[tokio::test]
    async fn a_pair_with_only_a_signature_a_phone_or_a_resent_from_is_found_with_that_line() {
        use postio_model::EvidenceKind;
        for (kind, values, expect) in [
            (
                EvidenceKind::Signature,
                ["ada moreno|finance lead|northwind partners"; 2],
                "The same signature on both",
            ),
            (
                EvidenceKind::Phone,
                ["+1 555 0119"; 2],
                "give the phone number +1 555 0119",
            ),
        ] {
            let database = test_support::memory().await;
            let (old, new) = two_addresses_one_person(&database).await;
            {
                let connection = database.connect().await.unwrap();
                let contacts = ContactRepository::new(&connection);
                for (message, value) in [old, new].into_iter().zip(values) {
                    contacts
                        .record_evidence(message, &[(kind, value.to_owned())])
                        .await
                        .unwrap();
                }
            }
            let groups = contact_review(&database).await;
            assert_eq!(groups.len(), 1, "{kind:?}: {groups:?}");
            assert!(
                groups[0].evidence.iter().any(|line| line.contains(expect)),
                "{kind:?}: {:?}",
                groups[0].evidence
            );
        }

        let database = test_support::memory().await;
        let (_old, new) = two_addresses_one_person(&database).await;
        {
            let connection = database.connect().await.unwrap();
            ContactRepository::new(&connection)
                .record_evidence(
                    new,
                    &[(EvidenceKind::Resent, "a.moreno@example.net".to_owned())],
                )
                .await
                .unwrap();
        }
        let groups = contact_review(&database).await;
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].evidence,
            ["ada@example.org forwards from a.moreno@example.net (Resent-From header)"]
        );
    }

    #[tokio::test]
    async fn an_imported_card_that_looks_like_someone_you_have_is_offered_in_the_review() {
        let database = test_support::memory().await;
        let card = |uid: &str, address: &str| {
            format!(
                "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:Ada Moreno\r\nEMAIL:{address}\r\nTEL:+1 555 0119\r\nEND:VCARD\r\n"
            )
        };
        let first = import_vcards(&database, card("urn:uuid:a", "ada@example.org").as_bytes())
            .await
            .expect("a first import");
        assert_eq!((first.people_created, first.look_alikes), (1, 0));
        assert!(
            contact_review(&database).await.is_empty(),
            "one Ada is no review"
        );

        let second = import_vcards(
            &database,
            card("urn:uuid:b", "a.moreno@example.net").as_bytes(),
        )
        .await
        .expect("a second import, another address");
        assert_eq!((second.people_created, second.look_alikes), (1, 1));
        let groups = contact_review(&database).await;
        assert_eq!(groups.len(), 1, "never a silent duplicate: {groups:?}");
        assert!(
            groups[0]
                .evidence
                .iter()
                .any(|line| line.contains("phone number")),
            "{:?}",
            groups[0].evidence
        );
    }

    #[tokio::test]
    async fn the_demos_import_file_updates_one_offers_one_and_adds_one() {
        let (database, _) = postio_demo::demo().await;
        let summary = import_vcards(&database, postio_demo::IMPORT_VCF.as_bytes())
            .await
            .expect("the demo's file imports");
        assert_eq!(
            (
                summary.people_created,
                summary.people_updated,
                summary.look_alikes
            ),
            (2, 1, 1),
            "{summary:?}"
        );
    }

    #[tokio::test]
    async fn the_demo_has_three_records_of_one_person_for_screen_07() {
        let (database, _) = postio_demo::demo().await;
        let groups = contact_review(&database).await;
        let ada = groups.first().expect("the demo has a review queue");
        assert!(
            ada.people.iter().any(|p| p.display_name() == "Ada Moreno"),
            "three records outrank two: {ada:?}"
        );
        assert_eq!(ada.people.len(), 3);
        assert!(
            ada.people[0].is_saved(),
            "the saved record comes first, to be kept"
        );
        assert_eq!(
            ada.evidence,
            [
                "The same signature on all three: \u{201c}Ada Moreno \u{b7} Finance lead, Northwind Partners\u{201d}",
                "a.moreno@example.net stopped when ada.moreno@northwind.example began, in the same 2 threads",
                "ada.m@example.org forwards from ada.moreno@northwind.example (Resent-From header)"
            ]
        );
    }

    #[tokio::test]
    async fn the_list_pages_by_tab_and_says_what_each_page_follows() {
        let database = test_support::memory().await;
        {
            let connection = database.connect().await.unwrap();
            let contacts = ContactRepository::new(&connection);
            for n in 0..5 {
                let address =
                    postio_model::EmailAddress::new(None::<String>, format!("p{n}@example.org"));
                contacts
                    .create(Some(&format!("Person {n}")), &[address])
                    .await
                    .unwrap();
            }
        }
        let tab = postio_model::ContactTab::All;
        let first = contacts_page(&database, tab, None, 0, 2).await.unwrap();
        assert_eq!(first.rows.len(), 2);
        assert_eq!(first.before, None, "the first row starts the list");
        let last = first.rows.last().unwrap();
        let next = contacts_page(&database, tab, Some((last.sort_key.clone(), last.id)), 0, 2)
            .await
            .unwrap();
        assert_eq!(next.before.as_deref(), Some(last.sort_key.as_str()));
        let jumped = contacts_page(&database, tab, None, 3, 2).await.unwrap();
        assert_eq!(jumped.rows.len(), 2);
        assert_eq!(
            jumped.before.as_deref(),
            Some(next.rows.first().unwrap().sort_key.as_str()),
            "a jump finds the row before it"
        );
        let found = contacts_filtered(&database, tab, "p3", 10).await.unwrap();
        assert_eq!(found.len(), 1);
        let overview = contacts_overview(&database).await.unwrap();
        assert_eq!(overview.total, 5);
    }

    #[tokio::test]
    async fn a_card_reads_as_of_the_twelve_months_ending_now() {
        let database = test_support::memory().await;
        old_mail(&database, "friend@example.org", 1).await;
        let person = contact_for_address(&database, "friend@example.org")
            .await
            .unwrap();
        use chrono::TimeZone as _;
        let now = chrono::Utc.with_ymd_and_hms(2026, 3, 9, 12, 0, 0).unwrap();
        let card = contact_card(&database, person.id, now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(card.stats.messages, 1);
        assert_eq!(card.stats.months.len(), 1);
        assert!(
            contact_card(&database, postio_model::ContactId::new(99), now)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_catch_up_reports_progress_in_batches_and_logs_counts_only() {
        let log = Captured::default();
        let _logging = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_ansi(false)
                .with_writer({
                    let log = log.clone();
                    move || log.clone()
                })
                .finish(),
        );
        let database = test_support::memory().await;
        for (n, from) in ["a@example.org", "b@example.org", "c@example.org"]
            .iter()
            .enumerate()
        {
            old_mail(&database, from, n as i64).await;
        }
        let mut seen = Vec::new();
        let made = catch_up(&database, 2, |progress| seen.push(progress))
            .await
            .expect("the catch-up runs");
        assert_eq!(made, 3);
        assert_eq!(
            seen,
            [
                CatchUpProgress { made: 2, total: 3 },
                CatchUpProgress { made: 3, total: 3 },
            ]
        );
        assert_eq!(
            catch_up(&database, 2, |_| {}).await.expect("again"),
            0,
            "a second start finds nothing to do"
        );
        let logged = String::from_utf8_lossy(&log.0.lock().expect("log")).into_owned();
        assert!(logged.contains("built contacts"), "{logged}");
        assert!(
            !logged.contains("example.org") && !logged.contains("Old Friend"),
            "{logged}"
        );
    }

    #[tokio::test]
    async fn the_kind_pass_sorts_senders_leaves_a_users_choice_and_runs_once_per_detector() {
        use chrono::TimeZone as _;
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        for day in 0..6 {
            let mut message = postio_model::Message::new(
                account.id,
                inbox,
                chrono::Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap()
                    + chrono::Duration::days(day),
            );
            message.from = vec![postio_model::EmailAddress::new(
                Some("Ledger Weekly"),
                "news@ledger.example",
            )];
            message.to = vec![account.address.clone()];
            message.promoted = Some(postio_model::promoted::PromotedHeaders {
                unsubscribe_offered: true,
                automation: postio_model::promoted::PRECEDENCE_BULK,
            });
            postio_storage::repository::MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("store");
        }
        old_mail(&database, "ada@example.org", 9).await;
        catch_up(&database, 10, |_| {})
            .await
            .expect("people are made");
        let contacts = ContactRepository::new(&connection);
        let kind = |address: &'static str| {
            let contacts = ContactRepository::new(&connection);
            async move {
                contacts
                    .by_address(address)
                    .await
                    .expect("read")
                    .expect("a person")
                    .kind
            }
        };
        let choosing = contacts
            .by_address("ada@example.org")
            .await
            .unwrap()
            .unwrap()
            .id;
        postio_storage::sql::execute(
            &connection,
            "UPDATE contacts SET kind = 'company', kind_source = 'user' WHERE id = ?1",
            postio_storage::sql::bind![choosing.get()],
        )
        .await
        .expect("the user chose");

        assert_eq!(detect_kinds(&database, 1).await.expect("the pass"), 1);
        assert_eq!(
            kind("news@ledger.example").await,
            postio_model::ContactKind::Newsletter
        );
        assert_eq!(
            kind("ada@example.org").await,
            postio_model::ContactKind::Company
        );
        assert_eq!(
            detect_kinds(&database, 1).await.expect("again"),
            0,
            "decided once per detector"
        );
    }

    #[tokio::test]
    async fn a_treatment_is_the_persons_and_forgetting_one_nobody_chose_makes_nobody() {
        let database = test_support::memory().await;
        old_mail(&database, "friend@example.org", 1).await;
        assert_eq!(treatments(&database).await.unwrap(), []);

        set_treatment(&database, "Friend@Example.org", Some("paper"))
            .await
            .unwrap();
        set_treatment(&database, "stranger@example.org", Some("app"))
            .await
            .unwrap();
        assert_eq!(
            treatments(&database).await.unwrap(),
            [
                ("friend@example.org".to_owned(), "paper".to_owned()),
                ("stranger@example.org".to_owned(), "app".to_owned())
            ]
        );
        set_treatment(&database, "friend@example.org", None)
            .await
            .unwrap();
        set_treatment(&database, "nobody@example.org", None)
            .await
            .unwrap();
        assert_eq!(treatments(&database).await.unwrap().len(), 1);
        assert!(
            set_treatment(&database, "friend@example.org", Some("sepia"))
                .await
                .is_err()
        );
        let connection = database.connect().await.unwrap();
        assert!(
            ContactRepository::new(&connection)
                .by_address("nobody@example.org")
                .await
                .unwrap()
                .is_none()
        );
    }
}
