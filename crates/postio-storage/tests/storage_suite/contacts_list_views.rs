//! The Contacts list's three views, its keyset pages, and its filter
//! (specs/005-contacts FR-003..FR-005, research R5).
//!
//! What a person sees in the default view is the people they made or
//! imported and the people they have written to; a newsletter they only
//! received is one toggle away, and still completes in the composer.

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Account, ContactListRow, ContactSource, ContactView, EmailAddress, MailboxId, Message,
};
use postio_storage::Connection;
use postio_storage::repository::{ContactCursor, ContactRepository, IdentityRepository};
use postio_storage::test_support;

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

fn address(name: Option<&str>, email: &str) -> EmailAddress {
    EmailAddress::new(name, email)
}

async fn setup() -> (
    postio_storage::Store,
    postio_storage::Checkout,
    Account,
    MailboxId,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

async fn received(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    from: EmailAddress,
) {
    let mut message = Message::new(account.id, inbox, at(0));
    message.from = vec![from];
    ContactRepository::new(connection)
        .record_message(&message, std::slice::from_ref(&account.address))
        .await
        .expect("record received");
}

async fn sent(connection: &Connection, account: &Account, inbox: MailboxId, to: EmailAddress) {
    let mut message = Message::new(account.id, inbox, at(1));
    message.from = vec![account.address.clone()];
    message.to = vec![to];
    ContactRepository::new(connection)
        .record_message(&message, std::slice::from_ref(&account.address))
        .await
        .expect("record sent");
}

fn names(rows: &[ContactListRow]) -> Vec<&str> {
    rows.iter().map(|row| row.name.as_str()).collect()
}

async fn view(connection: &Connection, view: ContactView) -> Vec<ContactListRow> {
    ContactRepository::new(connection)
        .page(view, None, 100)
        .await
        .expect("page")
}

#[tokio::test]
async fn the_default_view_is_the_people_the_user_made_or_wrote_to() {
    let (_db, connection, account, inbox) = setup().await;
    sent(
        &connection,
        &account,
        inbox,
        address(Some("Quinn Abara"), "quinn@example.net"),
    )
    .await;
    received(
        &connection,
        &account,
        inbox,
        address(Some("Weekly News"), "news@example.com"),
    )
    .await;
    ContactRepository::new(&connection)
        .create(Some("Grace Hopper"), &[address(None, "grace@example.org")])
        .await
        .expect("create grace");

    let written = view(&connection, ContactView::Written).await;
    assert_eq!(
        names(&written),
        ["Grace Hopper", "Quinn Abara"],
        "made and written-to, by name; the newsletter is not here"
    );
    assert_eq!(
        names(&view(&connection, ContactView::Everyone).await),
        ["Grace Hopper", "Quinn Abara", "Weekly News"],
        "everyone from mail includes the sender the user only received"
    );
    assert_eq!(
        ContactRepository::new(&connection)
            .complete("weekly", 8)
            .await
            .expect("complete")
            .len(),
        1,
        "and it still completes in the composer (FR-005)"
    );
}

#[tokio::test]
async fn each_row_says_whether_the_user_made_the_person() {
    let (_db, connection, account, inbox) = setup().await;
    sent(
        &connection,
        &account,
        inbox,
        address(Some("Quinn"), "quinn@example.net"),
    )
    .await;
    ContactRepository::new(&connection)
        .create(Some("Grace"), &[address(None, "grace@example.org")])
        .await
        .expect("create grace");

    let rows = view(&connection, ContactView::Written).await;
    let source = |name: &str| rows.iter().find(|r| r.name == name).expect(name).source;
    assert_eq!(
        source("Grace"),
        ContactSource::User,
        "the list marks her as made"
    );
    assert_eq!(source("Quinn"), ContactSource::Mail);
}

#[tokio::test]
async fn a_row_carries_what_the_list_draws() {
    let (_db, connection, _account, _inbox) = setup().await;
    ContactRepository::new(&connection)
        .create(
            Some("Ada Lovelace"),
            &[
                address(None, "ada@work.example"),
                address(None, "ada@home.example"),
            ],
        )
        .await
        .expect("create ada");

    let rows = view(&connection, ContactView::Written).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].preferred.as_deref(), Some("ada@work.example"));
    assert_eq!(rows[0].address_count, 2);
}

#[tokio::test]
async fn keyset_pages_follow_one_another_without_overlap_or_gap() {
    let (_db, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    for i in 0..25 {
        contacts
            .create(
                Some(&format!("Person {i:02}")),
                &[address(None, &format!("p{i}@example.com"))],
            )
            .await
            .expect("create");
    }

    let mut seen = Vec::new();
    let mut after: Option<ContactCursor> = None;
    loop {
        let page = contacts
            .page(ContactView::Written, after.as_ref(), 10)
            .await
            .expect("page");
        if page.is_empty() {
            break;
        }
        after = page.last().map(ContactCursor::after);
        seen.extend(page.into_iter().map(|row| row.name));
    }
    let expected: Vec<String> = (0..25).map(|i| format!("Person {i:02}")).collect();
    assert_eq!(seen, expected);
    assert_eq!(
        contacts.count(ContactView::Written).await.expect("count"),
        25
    );
}

#[tokio::test]
async fn the_filter_matches_any_name_the_organisation_and_any_part_of_any_address() {
    let (_db, connection, account, inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    contacts
        .create(
            Some("Ada Lovelace"),
            &[address(None, "countess@analytical.example")],
        )
        .await
        .expect("create ada");
    received(
        &connection,
        &account,
        inbox,
        address(Some("Charles Babbage"), "charles@engine.example"),
    )
    .await;

    let found = |text: &'static str| {
        let contacts = &contacts;
        async move {
            names(
                &contacts
                    .filtered(ContactView::Everyone, text, 500)
                    .await
                    .expect("filter"),
            )
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        found("love").await,
        ["Ada Lovelace"],
        "a surname, by prefix"
    );
    assert_eq!(
        found("countess").await,
        ["Ada Lovelace"],
        "an address's local part"
    );
    assert_eq!(
        found("analytical").await,
        ["Ada Lovelace"],
        "an address's domain"
    );
    assert_eq!(
        found("charles").await,
        ["Charles Babbage"],
        "the name the mail gave"
    );
    assert_eq!(
        found("ada love").await,
        ["Ada Lovelace"],
        "every word must match"
    );
    assert!(
        found("ada babbage").await.is_empty(),
        "…all of them, in one person"
    );
    assert!(found("zz").await.is_empty());
}

#[tokio::test]
async fn the_users_own_addresses_are_never_listed_even_if_seen_before_they_were_theirs() {
    // spec edge case: an address that becomes one of the user's identities
    // after it was recorded is the user's, not a contact.
    let (_db, connection, account, inbox) = setup().await;
    sent(
        &connection,
        &account,
        inbox,
        address(Some("Me At Work"), "me@work.example"),
    )
    .await;
    assert_eq!(view(&connection, ContactView::Written).await.len(), 1);

    let mut identity =
        postio_model::Identity::new(account.id, address(Some("Me At Work"), "me@work.example"));
    IdentityRepository::new(&connection)
        .create(&mut identity)
        .await
        .expect("add an identity");

    for kind in [ContactView::Written, ContactView::Everyone] {
        assert!(
            view(&connection, kind).await.is_empty(),
            "{kind:?} lists the user's own address as a contact"
        );
    }
}

#[tokio::test]
async fn the_detail_counts_messages_once_however_many_of_their_addresses_they_carry() {
    // FR-006: "how many distinct messages involve any of their addresses" --
    // a message to both of ada's addresses is one message, not two.
    let (_db, connection, account, inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .create(
            Some("Ada Lovelace"),
            &[
                address(None, "ada@work.example"),
                address(None, "ada@home.example"),
            ],
        )
        .await
        .expect("create ada");
    let mut both = Message::new(account.id, inbox, at(3));
    both.from = vec![address(Some("Someone"), "someone@example.org")];
    both.to = vec![
        address(None, "ada@work.example"),
        address(None, "ada@home.example"),
    ];
    postio_storage::repository::MessageRepository::new(&connection)
        .create(&mut both)
        .await
        .expect("store a message");
    let mut one = Message::new(account.id, inbox, at(4));
    one.from = vec![address(None, "ada@home.example")];
    postio_storage::repository::MessageRepository::new(&connection)
        .create(&mut one)
        .await
        .expect("store another");
    let group = postio_storage::repository::ContactGroupRepository::new(&connection)
        .create(&mut postio_model::ContactGroup::new("Analysts", at(0)))
        .await
        .expect("group");
    postio_storage::repository::ContactGroupRepository::new(&connection)
        .add_member(group, ada)
        .await
        .expect("member");

    let detail = contacts.detail(ada).await.expect("detail").expect("ada");
    assert_eq!(detail.person.addresses.len(), 2);
    assert_eq!(
        detail.messages, 2,
        "two messages, though three recipient rows"
    );
    assert_eq!(detail.groups, ["Analysts"]);
}
