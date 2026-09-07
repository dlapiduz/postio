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

/// An address at whichever provider the preset table happens to ship first.
///
/// Read out of the table rather than written down, for two reasons. The
/// repository is public and its fixtures may not name real domains, and —
/// more to the point — a test naming a provider would be the same mistake
/// the code is forbidden to make: providers are data, and this proves the
/// *table* is consulted rather than that one row exists.
fn an_address_at_a_known_provider() -> (String, String) {
    let preset = postio_account::discovery::presets()
        .first()
        .expect("the preset table ships at least one provider");
    let domain = preset
        .domains()
        .first()
        .expect("a preset claims at least one domain")
        .clone();
    (
        format!("someone@{domain}"),
        preset.display_name().to_owned(),
    )
}

#[test]
fn an_address_at_a_known_provider_is_recognised_and_says_which() {
    let (address, provider) = an_address_at_a_known_provider();
    let hint = provider_hint(address);

    assert_eq!(hint.provider, provider, "the strip names what it found");
    assert!(
        hint.verdict.contains(&provider),
        "and says so in a sentence: {}",
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
    let (address, _) = an_address_at_a_known_provider();
    let hint: ProviderHintFfi = provider_hint(address);
    assert!(hint.imap_port > 0);
    assert!(hint.smtp_port > 0);
    assert!(!hint.provider.is_empty());
}

// -- signing in through the browser (#1276) ----------------------------------

#[test]
fn signing_in_without_a_client_id_says_why_rather_than_failing_obscurely() {
    // ADR 0006 Q1: Postio ships no client id, and the reason is worth saying
    // out loud — a credential inside an open-source application is one every
    // user of it shares, and a provider that notices revokes it for all of
    // them at once.
    let (session, _) = a_session();

    let complaint = session
        .sign_in_with_browser("mara@example.com".to_owned(), "  ".to_owned(), None)
        .expect("a refusal");

    assert!(complaint.contains("client id"), "{complaint}");
    assert!(
        complaint.contains("shares") || complaint.contains("ships none"),
        "and says why: {complaint}"
    );
}

#[test]
fn a_provider_with_no_browser_sign_in_says_so_and_names_the_way_in() {
    // A dead end that suggests nothing is a dead end. IMAP with a password
    // is the route that works, and the message says so.
    let (session, _) = a_session();

    let complaint = session
        .sign_in_with_browser(
            "ada@ostwald.invalid".to_owned(),
            "the-users-own-client".to_owned(),
            None,
        )
        .expect("a refusal");

    assert!(complaint.contains("IMAP"), "{complaint}");
}

#[test]
fn nothing_is_in_flight_before_a_sign_in_starts() {
    let (session, _) = a_session();
    let progress = session.sign_in_progress();

    assert!(!progress.waiting);
    assert_eq!(progress.port, 0);
    assert!(progress.message.is_empty(), "and it says nothing at all");
}

#[test]
fn cancelling_when_nothing_is_signing_in_is_harmless() {
    // Closing the sheet always means this, whether a flow was running or not.
    let (session, _) = a_session();
    session.cancel_sign_in();
    assert!(!session.sign_in_progress().waiting);
}

#[test]
fn the_scopes_are_stated_in_plain_words_and_so_is_what_is_not_asked_for() {
    // A provider's consent screen lists what an application *may* do. Only
    // the application can say what it deliberately left out, and that is the
    // difference between asking permission and asking forgiveness.
    let scopes = postio_ffi::sign_in_scopes("someone@example.com".to_owned());

    assert!(scopes.asked_for.to_lowercase().contains("mail"));
    for absent in ["contacts", "calendar", "files"] {
        assert!(
            scopes.not_asked_for.to_lowercase().contains(absent),
            "{absent} is not named: {}",
            scopes.not_asked_for
        );
    }
}

#[test]
fn a_provider_that_offers_a_browser_sign_in_reports_the_scopes_its_row_asks_for() {
    // From the table, not from a list in Swift: what Postio requests is a
    // property of the provider row, and a second list would be a second
    // answer to what was consented to. The provider is found in the table
    // rather than named, for the reason `an_address_at_a_known_provider`
    // gives.
    let Some((domain, expected)) = postio_account::discovery::presets()
        .iter()
        .find_map(|preset| {
            let offer = preset.oauth()?;
            let domain = preset.domains().first()?.clone();
            (!offer.scopes.is_empty()).then(|| (domain, offer.scopes.clone()))
        })
    else {
        // No preset offers one: the assertion below would be vacuous, and
        // saying so is better than a test that passes by having nothing to
        // check.
        panic!("the preset table ships no provider with OAuth scopes");
    };

    let scopes = postio_ffi::sign_in_scopes(format!("someone@{domain}"));
    assert_eq!(scopes.requested, expected);
}
