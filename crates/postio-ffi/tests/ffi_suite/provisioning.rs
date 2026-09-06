//! Adding an account, across the boundary (#1279).
//!
//! Canvas 27's sheet asks for an address, says what it recognised, and offers
//! four routes. The recognising is the **preset table**, which is data rather
//! than code (`PRODUCT.md`: Postio is not built for any one provider) — so
//! what crosses is a verdict and a recommendation, never a branch per
//! provider written on the Swift side.

use postio_ffi::{ProviderHintFfi, RouteFfi, Session, SessionOptions, provider_hint};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

fn a_session() -> (std::sync::Arc<Session>, postio_storage::Database) {
    let database = test_support::memory();
    // A keyring of its own. Without this the suite writes into the developer's
    // *login keychain* — which it did once, on the first run of these tests,
    // and which is why the second run failed with "the item already exists".
    let secrets = std::sync::Arc::new(postio_account::secret::MemorySecretStore::default());
    let session =
        Session::open(SessionOptions::in_memory_with(database.clone()).with_secrets(secrets))
            .expect("a session over the store");
    (session, database)
}

#[test]
fn an_address_at_a_known_provider_is_recognised_and_says_which() {
    // The verdict strip. Named from the preset table's own display name, so
    // adding a provider is a data change and this keeps working.
    let hint = provider_hint("mara@gmail.com".to_owned());

    assert_eq!(hint.route, RouteFfi::Gmail);
    assert!(
        hint.verdict.contains("Gmail") || hint.verdict.contains("Google"),
        "the strip names what it found: {}",
        hint.verdict
    );
    assert!(!hint.imap_host.is_empty(), "and it knows where to connect");
}

#[test]
fn an_address_nobody_recognises_falls_to_imap_and_says_so() {
    // The ordinary case for a self-hosted domain, and not a failure: what it
    // must not do is guess `imap.<domain>`, which is how a client dials
    // somebody else's server.
    let hint = provider_hint("ada@ostwald.invalid".to_owned());

    assert_eq!(hint.route, RouteFfi::Imap);
    assert!(
        hint.imap_host.is_empty(),
        "no host is guessed from the domain"
    );
    assert!(
        !hint.verdict.is_empty(),
        "a strip that says nothing leaves the user with no idea what happens next"
    );
}

#[test]
fn something_that_is_not_an_address_is_not_recognised_as_anything() {
    let hint = provider_hint("not-an-address".to_owned());
    assert_eq!(hint.route, RouteFfi::Imap);
}

#[test]
fn adding_an_imap_account_writes_it_and_stores_the_password_out_of_reach() {
    let (session, database) = a_session();

    let complaint = session.add_imap_account(
        "ada@ostwald.invalid".to_owned(),
        "hunter2".to_owned(),
        "imap.ostwald.invalid".to_owned(),
        993,
        "smtp.ostwald.invalid".to_owned(),
        465,
    );

    assert_eq!(complaint, None, "no complaint");
    let connection = database.connection().expect("a connection");
    let accounts = AccountRepository::new(&connection)
        .list_enabled()
        .expect("a list");
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].address.address, "ada@ostwald.invalid");
    assert_eq!(accounts[0].incoming.host, "imap.ostwald.invalid");

    // The password is not in the account row, and never in config.toml: it
    // is in the keyring under the address (ADR 0014).
    let stored = format!("{:?}", accounts[0]);
    assert!(
        !stored.contains("hunter2"),
        "a password must not be in the row"
    );
}

#[test]
fn adding_the_same_address_twice_changes_nothing_and_says_nothing_broke() {
    // A re-run is deliberately inert: somebody who clicks Continue twice has
    // one account, not two, and no error to interpret.
    let (session, database) = a_session();
    let add = || {
        session.add_imap_account(
            "ada@ostwald.invalid".to_owned(),
            "hunter2".to_owned(),
            "imap.ostwald.invalid".to_owned(),
            993,
            "smtp.ostwald.invalid".to_owned(),
            465,
        )
    };

    assert_eq!(add(), None);
    assert_eq!(add(), None);

    let connection = database.connection().expect("a connection");
    assert_eq!(
        AccountRepository::new(&connection)
            .list_enabled()
            .expect("a list")
            .len(),
        1
    );
}

#[test]
fn an_account_with_no_host_is_refused_rather_than_written_half_made() {
    // The sheet can reach `Continue` with an empty host — the field is there
    // precisely because nothing else knows it — and an account that names no
    // server is one that fails later, at sync, as a connection error nobody
    // can act on.
    let (session, _) = a_session();

    let complaint = session
        .add_imap_account(
            "ada@ostwald.invalid".to_owned(),
            "hunter2".to_owned(),
            String::new(),
            993,
            "smtp.ostwald.invalid".to_owned(),
            465,
        )
        .expect("a refusal");

    assert!(
        complaint.to_lowercase().contains("server"),
        "and it says what is missing: {complaint}"
    );
}

#[test]
fn the_hint_is_a_record_the_sheet_can_draw_without_asking_again() {
    // Everything step 1 draws comes from one call: the verdict, which route
    // to pre-focus, and the servers to fill in.
    let hint: ProviderHintFfi = provider_hint("mara@gmail.com".to_owned());
    assert!(hint.imap_port > 0);
    assert!(hint.smtp_port > 0);
    assert!(!hint.provider.is_empty());
}
