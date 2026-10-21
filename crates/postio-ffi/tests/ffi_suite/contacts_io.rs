//! Importing and exporting contacts across the boundary (specs/011-focus-contacts
//! S11): the Mac hands over the bytes of a file the user chose and gets back
//! the sheet to show; an export hands back the file's text. An import
//! offers merges and never makes one.

use postio_ffi::{Session, SessionOptions};
use postio_storage::test_support;

fn card(uid: &str, name: &str, address: &str) -> String {
    format!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{name}\r\nEMAIL:{address}\r\nEND:VCARD\r\n"
    )
}

async fn session() -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_says_what_it_did_and_offers_the_review_for_a_look_alike() {
    let session = session().await;
    let s = session.clone();
    let first = tokio::task::spawn_blocking(move || {
        s.contacts_import(card("urn:uuid:a", "Ada Moreno", "ada@example.org").into_bytes())
    })
    .await
    .expect("imported");
    let sheet = first.sheet.expect("a sheet");
    assert_eq!(sheet.title, "Imported 1 contact");
    assert!(sheet.review.is_none(), "nothing to review yet");
    assert!(sheet.lines.last().unwrap().contains("cannot be undone"));

    let s = session.clone();
    let second = tokio::task::spawn_blocking(move || {
        s.contacts_import(card("urn:uuid:b", "Ada Moreno", "a.moreno@example.net").into_bytes())
    })
    .await
    .expect("imported");
    let sheet = second.sheet.expect("a sheet");
    let review = sheet.review.expect("a look-alike is offered");
    assert_eq!(review.label, "Review");
    assert!(sheet.lines.iter().any(|l| l.contains("Nothing was merged")));
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_export_hands_back_the_file_and_a_sheet_that_says_how_many() {
    let session = session().await;
    let s = session.clone();
    tokio::task::spawn_blocking(move || {
        s.contacts_import(card("urn:uuid:a", "Ada Moreno", "ada@example.org").into_bytes())
    })
    .await
    .expect("imported");
    let s = session.clone();
    let out = tokio::task::spawn_blocking(move || s.contacts_export())
        .await
        .expect("exported");
    let text = out.text.expect("a file");
    assert!(text.contains("BEGIN:VCARD") && text.contains("ada@example.org"));
    assert_eq!(out.sheet.expect("a sheet").title, "Exported 1 contact");
    assert_eq!(out.error, None);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_panels_say_where_the_file_goes() {
    let session = session().await;
    let words = session.contacts_panel_words();
    assert_eq!(words.export_name, "Contacts.vcf");
    assert!(words.import_message.contains("uploads nothing"));
    session.shutdown();
}
