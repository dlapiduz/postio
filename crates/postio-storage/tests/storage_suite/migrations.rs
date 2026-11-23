//! Opening a store at any schema in `tests/schemas/` must end where a fresh store begins, with what
//! it held still in it. A store no migration reaches is refused (`refuses_a_stale_schema.rs`).

use postio_storage::sql::{self, RowExt as _};
use postio_storage::{Store, schema, test_support};

const SCHEMAS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/schemas");

fn recorded() -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = std::fs::read_dir(SCHEMAS)
        .expect("tests/schemas reads")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "sql"))
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read_to_string(entry.path()).expect("a schema reads"),
            )
        })
        .collect();
    found.sort();
    found
}

/// Compared on facts: indexes and triggers by statement with spacing and `IF NOT EXISTS` removed (a
/// migration creates with it, a fresh store without).
async fn shape(store: &Store) -> Vec<String> {
    let connection = store.connect().await.expect("a connection");
    let objects = sql::all_unbounded(
        &connection,
        "SELECT type, name, coalesce(sql, '') FROM sqlite_schema
          WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        (),
        |row| Ok((row.text(0)?, row.text(1)?, row.text(2)?)),
    )
    .await
    .expect("the schema reads");
    let mut facts = Vec::new();
    for (kind, name, statement) in objects {
        if kind == "table" {
            let columns = sql::all_unbounded(
                &connection,
                &format!("PRAGMA table_info({name})"),
                (),
                |row| {
                    Ok(format!(
                        "{} {} notnull={} default={:?} pk={}",
                        row.text(1)?,
                        row.text(2)?,
                        row.int(3)?,
                        row.opt_text(4)?,
                        row.int(5)?,
                    ))
                },
            )
            .await
            .expect("the columns read");
            facts.push(format!("table {name}: {}", columns.join(", ")));
        } else {
            let statement = statement
                .replace("IF NOT EXISTS ", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            facts.push(format!("{kind} {name}: {statement}"));
        }
    }
    facts
}

const HELD: &str = "
INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port,
                      incoming_username, outgoing_host, outgoing_port,
                      outgoing_username, created_at)
     VALUES (7, 'Ada', 'ada@example.com', 'imap.example.com', 993,
             'ada@example.com', 'smtp.example.com', 587, 'ada@example.com', 1);
INSERT INTO operation_queue (account_id, op_type, created_at, updated_at)
     VALUES (7, 'archive', 1, 1);
";

#[tokio::test]
async fn every_recorded_schema_is_migrated_in_place_to_head() {
    let schemas = recorded();
    assert!(
        !schemas.is_empty(),
        "tests/schemas holds the schemas earlier builds stamped; with none, \
         nothing here is tested"
    );
    let fresh_dir = tempfile::tempdir().unwrap();
    let fresh = Store::open(fresh_dir.path().join("postio.db"), &test_support::key())
        .await
        .expect("a fresh store opens");
    let head = shape(&fresh).await;

    for (file, text) in schemas {
        let stamp = schema::fingerprint(&text);
        assert_eq!(
            file,
            format!("{:08x}.sql", stamp as i32 as u32),
            "a recorded schema is named for the fingerprint its text hashes \
             to, so a migration's `from` finds it"
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postio.db");
        Store::create_at_schema(&path, &test_support::key(), &text, HELD)
            .await
            .expect("a store at the earlier schema is written");

        let store = match Store::open(&path, &test_support::key()).await {
            Ok(store) => store,
            Err(error) => panic!(
                "a store stamped {file} has to open, migrated: an earlier \
                 build's store is the person's, and refusing it costs them \
                 everything only this file holds. Got: {error}"
            ),
        };
        assert_eq!(
            shape(&store).await,
            head,
            "a store migrated from {file} has to be shaped as a fresh one"
        );
        let connection = store.connect().await.expect("a connection");
        assert_eq!(
            sql::scalar(&connection, "PRAGMA user_version", ())
                .await
                .unwrap(),
            schema::FINGERPRINT,
            "a migrated store is stamped as this build's, or it migrates on \
             every open"
        );
        assert_eq!(
            sql::scalar(
                &connection,
                "SELECT count(*) FROM operation_queue WHERE account_id = 7",
                ()
            )
            .await
            .unwrap(),
            1,
            "the change the server has not heard about yet survives the \
             migration"
        );
        drop(connection);
        drop(store);
        Store::open(&path, &test_support::key())
            .await
            .expect("a migrated store opens again as this build's own");
    }
}

const HELD_CONTACTS: &str = "
INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port,
                      incoming_username, outgoing_host, outgoing_port,
                      outgoing_username, created_at)
     VALUES (1, 'Me', 'me@example.com', 'imap.example.com', 993,
             'me@example.com', 'smtp.example.com', 587, 'me@example.com', 1),
            (2, 'Work', 'me@example.net', 'imap.example.net', 993,
             'me@example.net', 'smtp.example.net', 587, 'me@example.net', 1);
INSERT INTO addresses (id, address, address_normalized)
     VALUES (10, 'Ada@Example.org', 'ada@example.org'),
            (11, 'bob@example.org', 'bob@example.org');
INSERT INTO correspondents (address_id, sent_count) VALUES (10, 4);
INSERT INTO contacts (id, account_id, name, address, address_name,
                      address_normalized, source, suppressed, uid, times_seen,
                      last_seen_at)
     VALUES (1, 1, 'Ada Lovelace', 'Ada@Example.org', 'A. L.',
             'ada@example.org', 'user', 0, 'urn:uuid:ada', 5, 100),
            (2, 2, NULL, 'Ada@Example.org', 'Ada L',
             'ada@example.org', 'mail', 0, NULL, 3, 200),
            (3, 1, NULL, 'bob@example.org', 'Bob',
             'bob@example.org', 'mail', 1, NULL, 1, 50),
            (4, 1, NULL, 'carol@example.org', NULL,
             'carol@example.org', 'mail', 0, NULL, 2, 60);
INSERT INTO contact_groups (id, account_id, name, created_at)
     VALUES (1, NULL, 'Book club', 1), (2, 1, 'book club', 2);
INSERT INTO contact_group_members (group_id, contact_id) VALUES (1, 1), (1, 3);
";

type Person = (String, Option<String>, String, String, i64, Option<String>);

#[tokio::test]
async fn the_contacts_of_the_build_before_become_people_who_own_addresses() {
    let before = recorded()
        .into_iter()
        .find(|(file, _)| file == "84e772c0.sql")
        .expect("the schema before contacts became people is recorded")
        .1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    Store::create_at_schema(&path, &test_support::key(), &before, HELD_CONTACTS)
        .await
        .expect("a store at the earlier schema is written");
    let store = Store::open(&path, &test_support::key())
        .await
        .expect("it migrates");
    let connection = store.connect().await.expect("a connection");

    let through: String = sql::first(
        &connection,
        "SELECT value FROM settings WHERE key = 'contacts.history_through'",
        (),
        |row| row.text(0),
    )
    .await
    .unwrap()
    .expect("the mark is written by the step");
    assert_eq!(through, "0", "this store held no mail");

    let people: Vec<Person> = sql::all_unbounded(
        &connection,
        "SELECT a.address_normalized, c.name, c.source, c.state, c.times_seen, c.uid
               FROM contacts c JOIN addresses a ON a.contact_id = c.id
              ORDER BY a.address_normalized",
        (),
        |row| {
            Ok((
                row.text(0)?,
                row.opt_text(1)?,
                row.text(2)?,
                row.text(3)?,
                row.int(4)?,
                row.opt_text(5)?,
            ))
        },
    )
    .await
    .unwrap();
    assert_eq!(
        people,
        [
            (
                "ada@example.org".to_owned(),
                Some("Ada Lovelace".to_owned()),
                "user".to_owned(),
                "live".to_owned(),
                8,
                Some("urn:uuid:ada".to_owned())
            ),
            (
                "bob@example.org".to_owned(),
                None,
                "mail".to_owned(),
                "deleted".to_owned(),
                1,
                None
            ),
            (
                "carol@example.org".to_owned(),
                None,
                "mail".to_owned(),
                "live".to_owned(),
                2,
                None
            ),
        ],
        "two rows for one address are one person, the user's name and source win, \
         and a suppressed address is a deleted person"
    );

    assert_eq!(
        sql::scalar(
            &connection,
            "SELECT count(*) FROM contact_sightings s
               JOIN addresses a ON a.id = s.address_id
              WHERE a.address_normalized = 'ada@example.org'",
            ()
        )
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        sql::scalar(
            &connection,
            "SELECT c.written FROM contacts c JOIN addresses a ON a.contact_id = c.id
              WHERE a.address_normalized = 'ada@example.org'",
            ()
        )
        .await
        .unwrap(),
        4
    );
    assert_eq!(
        sql::scalar(
            &connection,
            "SELECT count(*) FROM contacts c JOIN addresses a ON a.contact_id = c.id
              WHERE c.listed = 1 AND a.address_normalized = 'ada@example.org'",
            ()
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sql::scalar(
            &connection,
            "SELECT count(*) FROM contacts WHERE listed = 1",
            ()
        )
        .await
        .unwrap(),
        1
    );
    for (term, people) in [
        ("ada@example.org", 1),
        ("ada", 1),
        ("example.org", 3),
        ("ada lovelace", 1),
    ] {
        assert_eq!(
            sql::scalar(
                &connection,
                &format!("SELECT count(*) FROM contact_terms WHERE term = '{term}'"),
                ()
            )
            .await
            .unwrap(),
            people,
            "{term}"
        );
    }
    assert_eq!(
        sql::scalar(&connection, "SELECT count(*) FROM contact_groups", ())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sql::scalar(
            &connection,
            "SELECT count(*) FROM contact_group_members",
            ()
        )
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        sql::scalar(
            &connection,
            "SELECT count(*) FROM contacts c
              WHERE c.preferred_address IN (SELECT id FROM addresses WHERE contact_id = c.id)",
            ()
        )
        .await
        .unwrap(),
        3
    );
}

const HELD_PEOPLE: &str = "
INSERT INTO addresses (id, address, address_normalized)
     VALUES (1, 'ada@example.org', 'ada@example.org'),
            (2, 'bob@example.org', 'bob@example.org'),
            (3, 'carol@example.org', 'carol@example.org');
INSERT INTO contacts (id, name, source, state, preferred_address, written,
                      created_at, updated_at)
     VALUES (1, 'Ada', 'user', 'live', 1, 0, 500, 600),
            (2, 'Bob', 'import', 'live', 2, 0, 700, 800),
            (3, NULL, 'mail', 'live', 3, 4, 900, 900);
UPDATE addresses SET contact_id = id;
";

type Kinded = (i64, Option<i64>, String, String, i64, String);

#[tokio::test]
async fn people_the_user_asked_for_are_saved_and_everyone_is_a_detected_person() {
    let before = recorded()
        .into_iter()
        .find(|(file, _)| file == "d8313529.sql")
        .expect("the schema before Focus's Contacts is recorded")
        .1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    Store::create_at_schema(&path, &test_support::key(), &before, HELD_PEOPLE)
        .await
        .expect("a store at the earlier schema is written");
    let store = Store::open(&path, &test_support::key())
        .await
        .expect("it migrates");
    let connection = store.connect().await.expect("a connection");

    let rows: Vec<Kinded> = sql::all_unbounded(
        &connection,
        "SELECT id, saved_at, kind, kind_source, kind_version, show_as
           FROM contacts ORDER BY id",
        (),
        |row| {
            Ok((
                row.int(0)?,
                row.opt_int(1)?,
                row.text(2)?,
                row.text(3)?,
                row.int(4)?,
                row.text(5)?,
            ))
        },
    )
    .await
    .unwrap();
    let person = |id, saved| {
        (
            id,
            saved,
            "person".to_owned(),
            "detected".to_owned(),
            0,
            "full".to_owned(),
        )
    };
    assert_eq!(
        rows,
        [person(1, Some(500)), person(2, Some(700)), person(3, None),]
    );
    let marked: i64 = sql::scalar(
        &connection,
        "SELECT count(*) FROM addresses WHERE bounced_at IS NOT NULL OR label IS NOT NULL",
        (),
    )
    .await
    .unwrap();
    assert_eq!(marked, 0, "no address starts labelled or bounced");
}
