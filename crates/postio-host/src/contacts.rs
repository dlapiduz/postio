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
        joined = summary.joins.len(),
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
    let connection = database.connect().await?;
    Ok(
        postio_storage::repository::ContactSettingsRepository::new(&connection)
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
