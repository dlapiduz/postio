//! People for the mail that was here before contacts were: made on demand,
//! and by a background catch-up that never blocks anything
//! (specs/011-focus-contacts, plan section 4.2).

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Account, AddressId, ContactId, ContactKind, ContactSource, ContactView, EmailAddress,
    MailboxId, Message,
};
use postio_storage::repository::{ContactRepository, MessageRepository};
use postio_storage::test_support::counting::{counted_async, unbounded};
use postio_storage::{Checkout, Connection, test_support};

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

fn who(name: Option<&str>, address: &str) -> EmailAddress {
    EmailAddress::new(name, address)
}

/// Stores a message the way mail that predates contacts was stored: its
/// recipients and addresses are there, and nobody was made of them.
async fn stored(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    day: i64,
    from: EmailAddress,
    to: Vec<EmailAddress>,
) {
    let mut message = Message::new(account.id, inbox, at(day));
    message.from = vec![from];
    message.to = to;
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("store a message");
}

async fn address_of(connection: &Connection, email: &str) -> AddressId {
    let id: i64 = postio_storage::sql::scalar(
        connection,
        "SELECT id FROM addresses WHERE address_normalized = ?1",
        postio_storage::sql::bind![email],
    )
    .await
    .expect("the address is stored");
    AddressId::new(id)
}

async fn setup() -> (postio_storage::Store, Checkout, Account, MailboxId) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

#[tokio::test]
async fn an_address_with_mail_and_no_person_gets_one_on_demand_and_only_once() {
    let (_database, connection, account, inbox) = setup().await;
    let me = who(Some("Me"), &account.address.address);
    stored(
        &connection,
        &account,
        inbox,
        1,
        who(Some("Ada"), "ada@example.org"),
        vec![me.clone()],
    )
    .await;
    stored(
        &connection,
        &account,
        inbox,
        3,
        who(Some("Ada L."), "ada@example.org"),
        vec![me.clone()],
    )
    .await;
    stored(
        &connection,
        &account,
        inbox,
        5,
        me,
        vec![who(None, "ada@example.org")],
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    let ada = address_of(&connection, "ada@example.org").await;
    assert!(
        contacts
            .by_address("ada@example.org")
            .await
            .unwrap()
            .is_none()
    );

    let person = contacts
        .ensure_for_address(ada)
        .await
        .expect("a person is made");
    assert_eq!(
        contacts.ensure_for_address(ada).await.unwrap(),
        person,
        "never a second one"
    );

    let ada = contacts.get(person).await.unwrap().expect("stored");
    assert_eq!(ada.source, ContactSource::Mail);
    assert_eq!(ada.kind, ContactKind::Person);
    assert!(!ada.is_saved(), "making a person is not saving them");
    assert_eq!(
        ada.seen_name.as_deref(),
        Some("Ada L."),
        "the newest name the mail gave"
    );
    assert_eq!(ada.times_seen, 3, "three messages, once each");
    assert_eq!(ada.written, 1, "one of them was sent from my own address");
    assert_eq!(ada.last_seen_at, Some(at(5)));
    assert_eq!(ada.addresses.len(), 1);
    assert_eq!(
        contacts.count(ContactView::Written).await.unwrap(),
        1,
        "someone written to belongs in the default list, as one recorded at insert would"
    );
    assert_eq!(
        contacts.complete("ada", 5).await.unwrap().len(),
        1,
        "and completes, through the terms the same step wrote"
    );
}

#[tokio::test]
async fn the_users_own_address_is_never_made_a_person() {
    let (_database, connection, account, inbox) = setup().await;
    let me = who(None, &account.address.address);
    stored(
        &connection,
        &account,
        inbox,
        1,
        who(None, "ada@example.org"),
        vec![me],
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    let mine = address_of(&connection, &account.address.normalized()).await;

    assert!(contacts.ensure_for_address(mine).await.is_err());
    let batch = contacts.catch_up_batch(None, 50).await.unwrap();
    assert_eq!(batch.made, 1, "ada, and not me");
    assert!(
        contacts
            .by_address(&account.address.address)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_catch_up_walks_newest_first_in_batches_and_finishes() {
    let (_database, connection, account, inbox) = setup().await;
    let me = who(None, &account.address.address);
    for (day, who_) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
        stored(
            &connection,
            &account,
            inbox,
            day as i64,
            who(None, &format!("{who_}@example.org")),
            vec![me.clone()],
        )
        .await;
    }
    let contacts = ContactRepository::new(&connection);
    assert_eq!(contacts.unowned_count().await.unwrap(), 5);

    let first = contacts.catch_up_batch(None, 2).await.unwrap();
    assert_eq!(first.made, 2);
    assert!(!first.done);
    assert!(
        contacts
            .by_address("e@example.org")
            .await
            .unwrap()
            .is_some()
            && contacts
                .by_address("d@example.org")
                .await
                .unwrap()
                .is_some(),
        "the newest addresses first"
    );
    assert!(
        contacts
            .by_address("a@example.org")
            .await
            .unwrap()
            .is_none()
    );

    let mut before = first.last;
    let mut total = first.made;
    loop {
        let batch = contacts.catch_up_batch(before, 2).await.unwrap();
        total += batch.made;
        before = batch.last;
        if batch.done {
            break;
        }
    }
    assert_eq!(total, 5);
    assert_eq!(contacts.unowned_count().await.unwrap(), 0);
    let again = contacts.catch_up_batch(None, 2).await.unwrap();
    assert_eq!(
        (again.made, again.done),
        (0, true),
        "a second pass finds nothing"
    );
    let ids: Vec<ContactId> = contacts
        .people(100)
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids.len(), 5, "one person each, none twice");
}

#[tokio::test]
async fn a_catch_up_pass_with_nothing_to_do_is_one_read_and_no_scan() {
    let (_database, connection, account, inbox) = setup().await;
    stored(
        &connection,
        &account,
        inbox,
        1,
        who(None, "a@example.org"),
        vec![who(None, &account.address.address)],
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    contacts.catch_up_batch(None, 50).await.unwrap();

    let idle = counted_async(|| async {
        contacts.catch_up_batch(None, 50).await.unwrap();
    })
    .await;
    assert_eq!(
        idle.statements, 1,
        "the walk alone; this runs at every start"
    );

    let plan = postio_storage::test_support::counting::plan_steps(
        &connection,
        ContactRepository::explain_unowned(),
    )
    .await
    .expect("the walk plans");
    assert!(
        plan.iter()
            .any(|step| step.contains("idx_addresses_contact")),
        "the walk reaches the unowned addresses through their index: {plan:?}"
    );
    assert_eq!(
        unbounded(
            &connection,
            ContactRepository::explain_unowned(),
            &["addresses", "recipients", "messages"]
        )
        .await,
        Vec::<String>::new(),
        "the walk seeks the unowned addresses through idx_addresses_contact"
    );
}

#[tokio::test]
async fn making_a_person_costs_a_fixed_number_of_statements() {
    let (_database, connection, account, inbox) = setup().await;
    let me = who(None, &account.address.address);
    for n in 0..30 {
        stored(
            &connection,
            &account,
            inbox,
            n,
            who(None, "busy@example.org"),
            vec![me.clone()],
        )
        .await;
    }
    stored(
        &connection,
        &account,
        inbox,
        40,
        who(None, "quiet@example.org"),
        vec![me.clone()],
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    let busy = address_of(&connection, "busy@example.org").await;
    let quiet = address_of(&connection, "quiet@example.org").await;

    let few = counted_async(|| async {
        contacts.ensure_for_address(quiet).await.unwrap();
    })
    .await;
    let many = counted_async(|| async {
        contacts.ensure_for_address(busy).await.unwrap();
    })
    .await;
    assert_eq!(
        few.statements, many.statements,
        "30 messages cost the statements 1 does: they are folded in SQL, not read"
    );
    assert!(
        many.rows <= 40,
        "and read no row per message: read {}",
        many.rows
    );
}

#[tokio::test]
async fn new_mail_from_an_address_the_catch_up_has_not_reached_keeps_its_history() {
    let (_database, connection, account, inbox) = setup().await;
    let me = who(None, &account.address.address);
    for day in [1, 2] {
        stored(
            &connection,
            &account,
            inbox,
            day,
            who(Some("Ada"), "ada@example.org"),
            vec![me.clone()],
        )
        .await;
    }
    stored(
        &connection,
        &account,
        inbox,
        3,
        me.clone(),
        vec![who(None, "ada@example.org")],
    )
    .await;
    // The third arrives live: stored first, then recorded, as sync does.
    let mut live = Message::new(account.id, inbox, at(9));
    live.from = vec![who(Some("Ada"), "ada@example.org")];
    live.to = vec![me.clone()];
    MessageRepository::new(&connection)
        .create(&mut live)
        .await
        .unwrap();
    let contacts = ContactRepository::new(&connection);
    contacts
        .record_message(&live, std::slice::from_ref(&account.address))
        .await
        .unwrap();

    let ada = contacts
        .by_address("ada@example.org")
        .await
        .unwrap()
        .expect("a person");
    assert_eq!(
        ada.times_seen, 4,
        "three that were here, and the one that arrived"
    );
    assert_eq!(
        ada.written, 1,
        "the history includes the message the user sent"
    );
    assert_eq!(ada.last_seen_at, Some(at(9)));
    assert_eq!(
        contacts.unowned_count().await.unwrap(),
        0,
        "and the catch-up has nothing left of them"
    );

    // Recording the same kind of message again is the ordinary one-read path.
    let mut next = Message::new(account.id, inbox, at(10));
    next.from = vec![who(Some("Ada"), "ada@example.org")];
    next.to = vec![me];
    MessageRepository::new(&connection)
        .create(&mut next)
        .await
        .unwrap();
    contacts
        .record_message(&next, std::slice::from_ref(&account.address))
        .await
        .unwrap();
    assert_eq!(
        contacts
            .by_address("ada@example.org")
            .await
            .unwrap()
            .unwrap()
            .times_seen,
        5
    );
}
