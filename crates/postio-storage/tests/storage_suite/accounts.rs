//! Accounts and identities: create, read, update, delete, cascade.
//!
//! Written before the repositories existed.

use chrono::{TimeZone, Utc};

use postio_model::{
    Account, AccountId, AuthMethod, EmailAddress, Identity, IdentityId, Signature,
    TransportSecurity,
};
use postio_storage::repository::{AccountRepository, IdentityRepository, SignatureRepository};
use postio_storage::test_support;

fn an_account() -> Account {
    let mut account = Account::new(
        "iCloud",
        EmailAddress::new(Some("Ada Norwood"), "ada@icloud.example"),
    );
    account.incoming.host = "imap.mail.icloud.example".to_owned();
    account.outgoing.host = "smtp.mail.icloud.example".to_owned();
    account.auth = AuthMethod::AppPassword;
    account.created_at = Utc.with_ymd_and_hms(2026, 1, 5, 8, 30, 0).unwrap();
    account
}

fn an_identity(address: &str) -> Identity {
    Identity::new(
        AccountId::UNASSIGNED,
        EmailAddress::new(Some("Ada Norwood"), address),
    )
}

// ---------------------------------------------------------------------------
// Create and read
// ---------------------------------------------------------------------------

#[test]
fn an_account_round_trips_through_the_database() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    assert!(!account.id.is_assigned(), "not persisted yet");

    let id = accounts.create(&mut account).expect("create");

    assert!(id.is_assigned(), "the database hands out the id");
    assert_eq!(account.id, id, "and it is written back into the value");

    let stored = accounts.get(id).expect("get").expect("the account exists");
    assert_eq!(
        stored, account,
        "everything comes back exactly as it went in"
    );
    assert_eq!(stored.address.name.as_deref(), Some("Ada Norwood"));
    assert_eq!(stored.incoming.port, 993);
    assert_eq!(stored.incoming.security, TransportSecurity::Tls);
    assert_eq!(stored.outgoing.security, TransportSecurity::StartTls);
    assert_eq!(stored.auth, AuthMethod::AppPassword);
    assert_eq!(
        stored.created_at,
        Utc.with_ymd_and_hms(2026, 1, 5, 8, 30, 0).unwrap(),
        "timestamps survive the round trip through integer milliseconds"
    );
}

#[test]
fn enumerations_are_stored_with_the_spelling_the_model_documents() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    account.incoming.security = TransportSecurity::StartTls;
    account.outgoing.security = TransportSecurity::None;
    account.auth = AuthMethod::XOAuth2;
    accounts.create(&mut account).expect("create");

    let (incoming, outgoing, auth): (String, String, String) = connection
        .query_row(
            "SELECT incoming_security, outgoing_security, auth_method FROM accounts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("read the raw row");

    assert_eq!(incoming, TransportSecurity::StartTls.as_str());
    assert_eq!(outgoing, TransportSecurity::None.as_str());
    assert_eq!(auth, AuthMethod::XOAuth2.as_str());
}

#[test]
fn an_account_created_with_identities_gets_them_all_persisted() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    let mut work = an_identity("ada@work.example");
    work.is_default = true;
    work.signature = Some(Signature {
        id: Default::default(),
        name: String::new(),
        text: "-- \nAda".to_owned(),
        html: None,
    });
    account.identities = vec![an_identity("ada@icloud.example"), work];

    let id = accounts.create(&mut account).expect("create");

    for identity in &account.identities {
        assert!(identity.id.is_assigned(), "every identity got an id");
        assert_eq!(identity.account_id, id, "and knows its account");
    }

    let stored = accounts.get(id).expect("get").expect("the account");
    assert_eq!(stored.identities.len(), 2);
    assert_eq!(
        stored.identities[0].address.address, "ada@icloud.example",
        "identity order is preserved: it is the order the picker shows"
    );
    assert_eq!(
        stored
            .default_identity()
            .map(|identity| identity.address.address.as_str()),
        Some("ada@work.example")
    );
    assert_eq!(
        stored.identities[1]
            .signature
            .as_ref()
            .map(|s| s.text.as_str()),
        Some("-- \nAda")
    );
}

#[test]
fn reading_an_account_that_is_not_there_is_none_rather_than_an_error() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    assert!(accounts.get(AccountId::new(404)).expect("get").is_none());
    assert!(accounts.list().expect("list").is_empty());
}

#[test]
fn listing_returns_accounts_in_a_stable_order_and_can_filter_to_enabled() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut first = an_account();
    accounts.create(&mut first).expect("create");
    let mut second = Account::new(
        "Fastmail",
        EmailAddress::new(None::<String>, "b@example.com"),
    );
    second.enabled = false;
    accounts.create(&mut second).expect("create");

    let all = accounts.list().expect("list");
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].id, first.id, "creation order, stable across runs");
    assert_eq!(all[1].id, second.id);

    let enabled = accounts.list_enabled().expect("list enabled");
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].id, first.id, "a disabled account does not sync");
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

#[test]
fn updating_an_account_changes_its_row() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    let id = accounts.create(&mut account).expect("create");

    account.display_name = "iCloud (personal)".to_owned();
    account.incoming.host = "imap2.example.com".to_owned();
    account.enabled = false;
    accounts.update(&mut account).expect("update");

    let stored = accounts.get(id).expect("get").expect("the account");
    assert_eq!(stored.display_name, "iCloud (personal)");
    assert_eq!(stored.incoming.host, "imap2.example.com");
    assert!(!stored.enabled);
}

#[test]
fn updating_reconciles_the_identity_list() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    account.identities = vec![
        an_identity("one@example.com"),
        an_identity("two@example.com"),
    ];
    let id = accounts.create(&mut account).expect("create");
    let kept = account.identities[1].id;

    // Drop the first, keep the second, add a third — and reorder.
    let mut third = an_identity("three@example.com");
    third.is_default = true;
    account.identities = vec![third, account.identities[1].clone()];
    account.identities[1].display_name = "Renamed".to_owned();
    accounts.update(&mut account).expect("update");

    let stored = accounts.get(id).expect("get").expect("the account");
    let addresses: Vec<&str> = stored
        .identities
        .iter()
        .map(|identity| identity.address.address.as_str())
        .collect();
    assert_eq!(
        addresses,
        ["three@example.com", "two@example.com"],
        "the list is exactly what was handed in, in that order"
    );
    assert_eq!(
        stored.identities[1].id, kept,
        "an identity that stayed keeps its id, so drafts pointing at it survive"
    );
    assert_eq!(stored.identities[1].display_name, "Renamed");
    assert!(stored.identities[0].is_default);

    let orphans: i64 = connection
        .query_row("SELECT count(*) FROM identities", [], |row| row.get(0))
        .expect("count");
    assert_eq!(orphans, 2, "the removed identity's row is gone");
}

#[test]
fn updating_an_unpersisted_account_is_an_error_rather_than_a_silent_no_op() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    assert!(accounts.update(&mut an_account()).is_err());
}

// ---------------------------------------------------------------------------
// Identities on their own
// ---------------------------------------------------------------------------

#[test]
fn identities_can_be_managed_without_rewriting_the_account() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);
    let identities = IdentityRepository::new(&connection);

    let mut account = an_account();
    let account_id = accounts.create(&mut account).expect("create");

    let mut identity = an_identity("extra@example.com");
    identity.account_id = account_id;
    identity.reply_to = Some(EmailAddress::new(None::<String>, "replies@example.com"));
    let id = identities.create(&mut identity).expect("create");

    let stored = identities.get(id).expect("get").expect("the identity");
    assert_eq!(stored, identity);
    assert_eq!(stored.effective_reply_to().address, "replies@example.com");

    identity.display_name = "Extra".to_owned();
    identities.update(&identity).expect("update");
    assert_eq!(
        identities
            .get(id)
            .expect("get")
            .expect("still there")
            .display_name,
        "Extra"
    );

    assert_eq!(
        identities.list_for_account(account_id).expect("list").len(),
        1
    );
    assert!(identities.delete(id).expect("delete"));
    assert!(identities.get(id).expect("get").is_none());
    assert!(
        !identities.delete(id).expect("delete again"),
        "deleting what is gone is false, not an error"
    );
}

#[test]
fn an_account_has_at_most_one_default_identity() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);
    let identities = IdentityRepository::new(&connection);

    let mut account = an_account();
    let mut first = an_identity("one@example.com");
    first.is_default = true;
    account.identities = vec![first, an_identity("two@example.com")];
    let account_id = accounts.create(&mut account).expect("create");
    let second = account.identities[1].id;

    identities
        .set_default(account_id, second)
        .expect("move the default");

    let stored = accounts.get(account_id).expect("get").expect("the account");
    let defaults: Vec<IdentityId> = stored
        .identities
        .iter()
        .filter(|identity| identity.is_default)
        .map(|identity| identity.id)
        .collect();
    assert_eq!(defaults, [second], "exactly one, and it moved");
}

// ---------------------------------------------------------------------------
// Delete and cascade
// ---------------------------------------------------------------------------

#[test]
fn deleting_an_account_takes_everything_that_hangs_off_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    account.identities = vec![an_identity("one@example.com")];
    let id = accounts.create(&mut account).expect("create");

    connection
        .execute_batch(
            "INSERT INTO mailboxes (id, account_id, name, path) VALUES (1, 1, 'INBOX', 'INBOX');
             INSERT INTO sync_state (mailbox_id, account_id, uid_validity) VALUES (1, 1, 42);
             INSERT INTO messages (id, account_id, mailbox_id, received_at) VALUES (1, 1, 1, 0);
             INSERT INTO labels (id, account_id, name) VALUES (1, 1, 'Work');
             INSERT INTO message_labels (message_id, label_id) VALUES (1, 1);
             INSERT INTO threads (id, account_id) VALUES (1, 1);
             INSERT INTO operation_queue (account_id, op_type, created_at, updated_at)
                 VALUES (1, 'flag', 0, 0);",
        )
        .expect("seed everything that references an account");

    assert!(accounts.delete(id).expect("delete"));

    for table in [
        "accounts",
        "identities",
        "mailboxes",
        "sync_state",
        "messages",
        "labels",
        "message_labels",
        "threads",
        "operation_queue",
    ] {
        let remaining: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(remaining, 0, "{table} should have been cascaded away");
    }

    assert!(
        !accounts.delete(id).expect("delete again"),
        "deleting what is gone is false, not an error"
    );
}

// ---------------------------------------------------------------------------
// Enable, disable, and pending deletion (#464, ADR 0005 Q6a)
// ---------------------------------------------------------------------------

#[test]
fn set_enabled_flips_only_that_column() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    account.display_name = "before".to_owned();
    let id = accounts.create(&mut account).expect("create");
    assert!(accounts.get(id).unwrap().unwrap().enabled);

    assert!(accounts.set_enabled(id, false).expect("disable"));
    let stored = accounts.get(id).expect("get").expect("still there");
    assert!(!stored.enabled);
    assert_eq!(
        stored.display_name, "before",
        "set_enabled must not touch anything else about the row"
    );

    assert!(accounts.set_enabled(id, true).expect("re-enable"));
    assert!(accounts.get(id).unwrap().unwrap().enabled);
}

#[test]
fn set_enabled_on_a_missing_account_is_false_not_an_error() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    assert!(!accounts.set_enabled(AccountId::new(404), false).unwrap());
}

#[test]
fn marking_for_deletion_is_reversible_until_it_is_reaped() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    let id = accounts.create(&mut account).expect("create");
    assert!(!accounts.get(id).unwrap().unwrap().pending_deletion);

    assert!(accounts.mark_pending_deletion(id).expect("mark"));
    assert!(
        accounts.get(id).unwrap().unwrap().pending_deletion,
        "marked, but not yet actually deleted"
    );

    assert!(accounts.restore(id).expect("restore"));
    assert!(
        !accounts.get(id).unwrap().unwrap().pending_deletion,
        "undo clears the mark, and the account is untouched"
    );
}

#[test]
fn a_marked_account_is_excluded_from_the_enabled_list_even_before_it_is_reaped() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut kept = an_account();
    accounts.create(&mut kept).expect("create");
    let mut leaving = Account::new(
        "Fastmail",
        EmailAddress::new(None::<String>, "b@example.com"),
    );
    let leaving_id = accounts.create(&mut leaving).expect("create");
    accounts.mark_pending_deletion(leaving_id).expect("mark");

    let enabled = accounts.list_enabled().expect("list enabled");
    assert_eq!(
        enabled.len(),
        1,
        "no engine should start against an account on its way out: {enabled:?}"
    );
    assert_eq!(enabled[0].id, kept.id);
}

#[test]
fn reaping_deletes_every_marked_account_and_leaves_the_rest() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut kept = an_account();
    accounts.create(&mut kept).expect("create");
    let mut leaving = Account::new(
        "Fastmail",
        EmailAddress::new(None::<String>, "b@example.com"),
    );
    let leaving_id = accounts.create(&mut leaving).expect("create");
    accounts.mark_pending_deletion(leaving_id).expect("mark");

    let reaped = accounts.reap_pending_deletions().expect("reap");
    assert_eq!(reaped, vec![leaving_id]);

    assert!(accounts.get(leaving_id).expect("get").is_none());
    assert!(accounts.get(kept.id).expect("get").is_some());
}

#[test]
fn reaping_cascades_exactly_like_an_ordinary_delete() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    let id = accounts.create(&mut account).expect("create");
    connection
        .execute(
            "INSERT INTO mailboxes (id, account_id, name, path) VALUES (1, 1, 'INBOX', 'INBOX')",
            [],
        )
        .expect("seed a mailbox");
    accounts.mark_pending_deletion(id).expect("mark");

    accounts.reap_pending_deletions().expect("reap");

    let remaining: i64 = connection
        .query_row("SELECT count(*) FROM mailboxes", [], |row| row.get(0))
        .expect("count");
    assert_eq!(remaining, 0, "reaping did not cascade");
}

#[test]
fn reaping_with_nothing_marked_deletes_nothing() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);

    let mut account = an_account();
    accounts.create(&mut account).expect("create");

    assert_eq!(accounts.reap_pending_deletions().expect("reap"), vec![]);
    assert!(accounts.get(account.id).expect("get").is_some());
}

#[test]
fn an_accounts_named_signatures_round_trip_and_arrive_with_it() {
    // #12: a signature is the account's, named, and chosen per message — not
    // a property of the one identity that happens to be selected.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);
    let mut account = an_account();
    accounts.create(&mut account).expect("create");

    let signatures = SignatureRepository::new(&connection);
    let mut long = Signature::new("Long", "Lena Tomlin\nPostio")
        .with_html("<p><strong>Lena Tomlin</strong><br>Postio</p>");
    let mut short = Signature::new("Short", "— Lena");
    signatures.create(account.id, &mut long).expect("create");
    signatures.create(account.id, &mut short).expect("create");
    assert!(long.id.is_assigned());

    // They arrive with the account, in picker order.
    let loaded = accounts
        .get(account.id)
        .expect("get")
        .expect("the account is there");
    assert_eq!(
        loaded
            .signatures
            .iter()
            .map(|signature| signature.name.as_str())
            .collect::<Vec<_>>(),
        ["Long", "Short"]
    );
    assert_eq!(
        loaded.signatures[0].html.as_deref(),
        Some("<p><strong>Lena Tomlin</strong><br>Postio</p>"),
        "the rich variant is part of the record"
    );

    // Renaming and rewriting one is an update, not a second row.
    long.name = "Full".to_owned();
    long.text = "Lena Tomlin".to_owned();
    signatures.update(&long).expect("update");
    let loaded = accounts.get(account.id).expect("get").expect("still there");
    assert_eq!(loaded.signatures.len(), 2);
    assert_eq!(loaded.signatures[0].name, "Full");
    assert_eq!(loaded.signatures[0].text, "Lena Tomlin");

    // And deleting one leaves the other.
    assert!(signatures.delete(short.id).expect("delete"));
    let loaded = accounts.get(account.id).expect("get").expect("still there");
    assert_eq!(loaded.signatures.len(), 1);
}

#[test]
fn two_accounts_never_see_each_others_signatures() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);
    let mut mine = an_account();
    accounts.create(&mut mine).expect("create");
    let mut theirs = Account::new(
        "Other",
        EmailAddress::new(None::<String>, "other@example.net"),
    );
    accounts.create(&mut theirs).expect("create");

    let signatures = SignatureRepository::new(&connection);
    signatures
        .create(mine.id, &mut Signature::new("Work", "Lena"))
        .expect("create");
    signatures
        .create(theirs.id, &mut Signature::new("Work", "Someone else"))
        .expect("create");

    // The same name on both accounts is fine; the uniqueness is per account.
    let loaded = accounts.get(mine.id).expect("get").expect("there");
    assert_eq!(loaded.signatures.len(), 1);
    assert_eq!(loaded.signatures[0].text, "Lena");
}

#[test]
fn oauth_composition_data_round_trips_and_stays_optional() {
    // #534: what the engine needs at every launch to rebuild an OAuth
    // account's token source. Optional twice over — password accounts and
    // broker-fed OAuth accounts both carry none.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let mut account = test_support::account(&connection);
    assert_eq!(account.oauth, None, "a password account carries no client");

    account.auth = postio_model::account::AuthMethod::XOAuth2;
    account.oauth = Some(postio_model::account::OAuthConfig {
        client_id: "postio-desktop.apps.example".to_string(),
        token_url: "https://auth.example.com/token".to_string(),
        authorize_url: "https://auth.example.com/authorize".to_string(),
        scopes: "https://mail.example.com/".to_string(),
    });
    AccountRepository::new(&connection)
        .update(&mut account)
        .expect("update");

    let read = AccountRepository::new(&connection)
        .get(account.id)
        .expect("read")
        .expect("the account");
    assert_eq!(read.auth, postio_model::account::AuthMethod::XOAuth2);
    assert_eq!(
        read.oauth.expect("the client survived").token_url,
        "https://auth.example.com/token",
        "startup rebuilds the token source from this, offline included"
    );
}

#[test]
fn the_backend_choice_round_trips_and_defaults_to_imap() {
    // ADR 0018 Q5: the backend is chosen at add-account time from the
    // preset row's preference and stored on the account; engine::start
    // reads it back every launch. Imap for every account that predates
    // the column.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let mut account = test_support::account(&connection);
    assert_eq!(
        account.backend,
        postio_model::account::Backend::Imap,
        "the default is the backend every existing account uses"
    );

    account.backend = postio_model::account::Backend::Jmap {
        session_url: "https://api.example.com/jmap/session/".to_string(),
    };
    AccountRepository::new(&connection)
        .update(&mut account)
        .expect("update");

    let read = AccountRepository::new(&connection)
        .get(account.id)
        .expect("read")
        .expect("the account");
    assert_eq!(
        read.backend,
        postio_model::account::Backend::Jmap {
            session_url: "https://api.example.com/jmap/session/".to_string(),
        }
    );
}

#[test]
fn two_signatures_in_one_account_cannot_share_a_name() {
    // `signatures.name` is documented "unique per account so the picker never
    // offers two entries a person cannot tell apart", and `idx_signatures_name`
    // is what makes that true rather than aspirational. Asserted because a
    // constraint nothing exercises is one a later migration can drop without
    // anybody noticing — and #1086 is about to put a *creation* flow in front
    // of it, where a duplicate stops being hypothetical.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);
    let mut account = an_account();
    accounts.create(&mut account).expect("create");

    let signatures = SignatureRepository::new(&connection);
    let mut first = Signature::new("Work", "— Lena");
    signatures.create(account.id, &mut first).expect("create");

    let mut clash = Signature::new("Work", "— Lena Tomlin");
    signatures
        .create(account.id, &mut clash)
        .expect_err("a second signature called Work must not be stored");

    // The first is untouched: a refused insert is not a partial one.
    let loaded = accounts.get(account.id).expect("get").expect("there");
    assert_eq!(loaded.signatures.len(), 1);
    assert_eq!(loaded.signatures[0].text, "— Lena");
}

#[test]
fn the_same_name_in_a_different_account_is_fine() {
    // The other half of "unique *per account*": two people's mail on one
    // machine both having a "Work" signature is the ordinary case, and a
    // global constraint would refuse the second.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);
    let mut mine = an_account();
    accounts.create(&mut mine).expect("create");
    let mut theirs = Account::new(
        "Other",
        EmailAddress::new(Some("Grace"), "grace@example.net"),
    );
    accounts.create(&mut theirs).expect("create");

    let signatures = SignatureRepository::new(&connection);
    signatures
        .create(mine.id, &mut Signature::new("Work", "— Lena"))
        .expect("create");
    signatures
        .create(theirs.id, &mut Signature::new("Work", "— Grace"))
        .expect("the same name in another account is a different signature");
}

#[test]
fn deleting_the_default_signature_leaves_no_dangling_reference() {
    // #1086's fourth criterion. `accounts.default_signature_id` is
    // `REFERENCES signatures(id) ON DELETE SET NULL`, so the row clears
    // itself — but only with foreign keys actually on, which is exactly the
    // kind of thing that is true until a connection somewhere forgets to
    // enable them.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let accounts = AccountRepository::new(&connection);
    let mut account = an_account();
    accounts.create(&mut account).expect("create");

    let signatures = SignatureRepository::new(&connection);
    let mut work = Signature::new("Work", "— Lena");
    let mut brief = Signature::new("Brief", "— L");
    signatures.create(account.id, &mut work).expect("create");
    signatures.create(account.id, &mut brief).expect("create");

    account.default_signature_id = Some(work.id);
    accounts.update(&mut account).expect("update");
    let loaded = accounts.get(account.id).expect("get").expect("there");
    assert_eq!(
        loaded.default_signature_id,
        Some(work.id),
        "the fixture has to start with a default, or the assertion below is vacuous"
    );

    assert!(signatures.delete(work.id).expect("delete"));

    let loaded = accounts.get(account.id).expect("get").expect("there");
    assert_eq!(
        loaded.default_signature_id, None,
        "the account still points at a signature that no longer exists, so \
         every reader of it is holding an id that resolves to nothing"
    );
    assert_eq!(
        loaded.signatures.len(),
        1,
        "and the one that was not deleted is still there"
    );
}
