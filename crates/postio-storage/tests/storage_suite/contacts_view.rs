//! What the Contacts view reads (specs/011-focus-contacts S7): the list by
//! kind tab, the strip's counts and review queue, and a card in one go.
//! Every read is a seek over the kind index, the counts or the address
//! rings, which the plans below hold to.

use postio_model::{ContactArrival, ContactKind, ContactTab, ContactView, month_key};
use postio_storage::repository::{ContactCursor, ContactRepository};
use postio_storage::test_support::counting::unbounded;

use super::contact_stats::{at, mail, person, setup};

async fn kind_of(connection: &postio_storage::Checkout, email: &str, kind: &str) {
    postio_storage::sql::execute(
        connection,
        "UPDATE contacts SET kind = ?2 WHERE id = (SELECT contact_id FROM addresses WHERE address_normalized = ?1)",
        postio_storage::sql::bind![email, kind],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_tab_lists_its_kinds_by_name_and_a_page_seeks_past_the_last() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    for (n, (email, kind)) in [
        ("ada@example.org", "person"),
        ("admin@lists.example", "list"),
        ("news@field.example", "newsletter"),
        ("bot@city.example", "automated"),
        ("ben@example.org", "person"),
        ("finance@lists.example", "list"),
    ]
    .iter()
    .enumerate()
    {
        mail(
            &connection,
            &account,
            inbox,
            at(2026, 1, 1 + n as u32),
            email,
            &me,
            true,
        )
        .await;
        kind_of(&connection, email, kind).await;
    }
    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET name = (SELECT address FROM addresses WHERE contact_id = contacts.id LIMIT 1), sort_key = (SELECT address_normalized FROM addresses WHERE contact_id = contacts.id LIMIT 1)",
        (),
    )
    .await
    .unwrap();
    let contacts = ContactRepository::new(&connection);

    let names = |rows: &[postio_model::ContactListRow]| {
        rows.iter().map(|row| row.name.clone()).collect::<Vec<_>>()
    };
    let all = contacts
        .page_kinds(ContactTab::All.kinds(), None, 0, 10)
        .await
        .unwrap();
    assert_eq!(all.len(), 6);
    let lists = contacts
        .page_kinds(ContactTab::Lists.kinds(), None, 0, 10)
        .await
        .unwrap();
    assert_eq!(
        names(&lists),
        [
            "admin@lists.example",
            "finance@lists.example",
            "news@field.example"
        ],
        "lists and newsletters, merged into one order"
    );
    assert_eq!(
        lists.iter().map(|row| row.kind).collect::<Vec<_>>(),
        [
            ContactKind::List,
            ContactKind::List,
            ContactKind::Newsletter
        ]
    );
    let first = contacts
        .page_kinds(ContactTab::Lists.kinds(), None, 0, 2)
        .await
        .unwrap();
    let next = contacts
        .page_kinds(
            ContactTab::Lists.kinds(),
            first.last().map(ContactCursor::after).as_ref(),
            0,
            2,
        )
        .await
        .unwrap();
    assert_eq!(names(&next), ["news@field.example"]);
    let skipped = contacts
        .page_kinds(ContactTab::Lists.kinds(), None, 1, 2)
        .await
        .unwrap();
    assert_eq!(
        names(&skipped),
        ["finance@lists.example", "news@field.example"]
    );
    assert_eq!(
        names(
            &contacts
                .page_kinds(ContactTab::Automated.kinds(), None, 0, 5)
                .await
                .unwrap()
        ),
        ["bot@city.example"]
    );

    let filtered = contacts
        .filtered_kinds(ContactTab::Lists.kinds(), "fin", 10)
        .await
        .unwrap();
    assert_eq!(
        names(&filtered),
        ["finance@lists.example"],
        "a filter within the tab"
    );

    for (name, sql) in postio_storage::test_support::contact_list_statements()
        .into_iter()
        .filter(|(name, _)| name.starts_with("kind"))
    {
        assert_eq!(
            unbounded(&connection, &sql, &["contacts"]).await,
            Vec::<String>::new(),
            "{name} is a seek over the kind index: {sql}"
        );
    }
    let _ = ContactView::Everyone;
}

#[tokio::test]
async fn a_row_says_what_a_tag_and_a_count_line_need() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    mail(
        &connection,
        &account,
        inbox,
        at(2026, 1, 1),
        "ada@example.org",
        &me,
        true,
    )
    .await;
    mail(
        &connection,
        &account,
        inbox,
        at(2026, 1, 2),
        "ada@example.org",
        &me,
        true,
    )
    .await;
    let ada = person(&connection, "ada@example.org").await;
    postio_storage::sql::execute(
        &connection,
        "INSERT INTO contact_settings (contact_id, arrival, digest_cadence, notify, html_treatment)
         VALUES (?1, 'digest', 'weekly', 1, 'paper')",
        [ada.get()],
    )
    .await
    .unwrap();
    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET saved_at = 5 WHERE id = ?1",
        [ada.get()],
    )
    .await
    .unwrap();

    let rows = ContactRepository::new(&connection)
        .page_kinds(ContactTab::All.kinds(), None, 0, 5)
        .await
        .unwrap();
    let row = &rows[0];
    assert_eq!(row.messages, 2);
    assert!(row.saved && row.notify && row.html_original);
    assert_eq!(row.arrival, Some(ContactArrival::Digest));
    assert_eq!(row.digest_cadence.as_deref(), Some("weekly"));
}

#[tokio::test]
async fn the_strip_counts_come_from_the_counts_table_and_the_queue_is_capped() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    for (n, email) in ["a@example.org", "b@example.org", "c@example.org"]
        .iter()
        .enumerate()
    {
        mail(
            &connection,
            &account,
            inbox,
            at(2026, 1, 1 + n as u32),
            email,
            &me,
            true,
        )
        .await;
    }
    kind_of(&connection, "c@example.org", "company").await;
    let contacts = ContactRepository::new(&connection);
    let overview = contacts.overview(true).await.unwrap();
    assert_eq!(overview.total, 3);
    assert_eq!(overview.saved, 0);
    assert_eq!(
        overview.tabs,
        [
            (ContactTab::All, 3),
            (ContactTab::People, 2),
            (ContactTab::Companies, 1),
            (ContactTab::Lists, 0),
            (ContactTab::Automated, 0),
        ]
    );
    assert!(
        overview.review.is_empty(),
        "nothing to look at yet: {:?}",
        overview.review
    );
    assert_eq!(overview.building, None);

    // Someone written to often and never saved is worth a look.
    for day in 1..=3 {
        mail(
            &connection,
            &account,
            inbox,
            at(2026, 2, day),
            &me,
            "a@example.org",
            true,
        )
        .await;
    }
    let overview = contacts.overview(true).await.unwrap();
    assert_eq!(overview.review.frequent_unsaved, 1);
    assert!(!overview.review.is_empty());
}

#[tokio::test]
async fn a_card_is_read_whole_from_the_rings() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    let first = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 1),
        "news@field.example",
        &me,
        true,
    )
    .await;
    let second = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 8),
        "news@field.example",
        &me,
        false,
    )
    .await;
    for (id, name, mime) in [
        (first, "Atlas-Q3-budget.xlsx", "application/vnd.ms-excel"),
        (second, "invoice.pdf", "application/pdf"),
    ] {
        postio_storage::sql::execute(
            &connection,
            "INSERT INTO attachments (message_id, filename, mime_type) VALUES (?1, ?2, ?3)",
            postio_storage::sql::bind![id.get(), name, mime],
        )
        .await
        .unwrap();
    }
    postio_storage::sql::execute(
        &connection,
        "UPDATE messages SET subject = 'The quiet season' WHERE id = ?1",
        [second.get()],
    )
    .await
    .unwrap();
    let sender = person(&connection, "news@field.example").await;
    let contacts = ContactRepository::new(&connection);

    let card = contacts
        .card(sender, month_key(at(2026, 1, 1)))
        .await
        .unwrap()
        .expect("a card");
    assert_eq!(card.stats.messages, 2);
    assert_eq!(card.person.id, sender);
    assert_eq!(
        card.files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>(),
        ["invoice.pdf", "Atlas-Q3-budget.xlsx"],
        "newest first"
    );
    assert_eq!(
        card.issues
            .iter()
            .map(|issue| (issue.subject.as_str(), issue.opened))
            .collect::<Vec<_>>(),
        [("The quiet season", false), ("(no subject)", true)]
    );
    assert_eq!(card.default_arrival, ContactArrival::Inbox);
    assert!(
        contacts
            .card(postio_model::ContactId::new(9999), 0)
            .await
            .unwrap()
            .is_none()
    );

    for sql in ContactRepository::explain_card_recent(1) {
        assert_eq!(
            unbounded(
                &connection,
                &sql,
                &["address_recent", "attachments", "messages"]
            )
            .await,
            Vec::<String>::new(),
            "bounded by the ring: {sql}"
        );
    }
}
