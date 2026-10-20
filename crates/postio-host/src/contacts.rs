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
}
