//! The migration runner's contract.
//!
//! Written before the runner existed. Every acceptance criterion on the bead
//! ("fresh DB migrates to head", "re-running is a no-op", "every migration
//! applies cleanly in order") has a test here.

use rusqlite::Connection;

use postio_storage::migrations::{self, Migration};
use postio_storage::{Error, migrate, schema_version};

/// A fresh, empty in-memory database.
fn empty() -> Connection {
    let connection = Connection::open_in_memory().expect("in-memory sqlite");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("enable foreign keys");
    connection
}

/// A migrated in-memory database. Not the shared test harness — that is a
/// separate bead; this is only what these tests need.
fn migrated() -> Connection {
    let mut connection = empty();
    migrate(&mut connection).expect("migrate to head");
    connection
}

fn table_names(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("prepare");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect")
}

/// Whether the plan resolves the query through an index rather than a scan.
/// SQLite says "USING INDEX" or, when the index answers the query on its own,
/// "USING COVERING INDEX"; both are what we are asking for.
fn uses_index(plan: &str) -> bool {
    plan.contains("USING INDEX") || plan.contains("USING COVERING INDEX")
}

fn query_plan(connection: &Connection, sql: &str) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap_or_else(|error| panic!("prepare {sql}: {error}"));
    let rows = statement
        .query_map([], |row| row.get::<_, String>(3))
        .expect("query plan")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect plan");
    rows.join("\n")
}

// ---------------------------------------------------------------------------
// Acceptance: fresh DB migrates to head
// ---------------------------------------------------------------------------

#[test]
fn fresh_database_migrates_to_head() {
    let mut connection = empty();
    assert_eq!(
        schema_version(&connection).expect("version of an empty db"),
        0,
        "an empty database is at version 0"
    );

    let report = migrate(&mut connection).expect("migrate");

    assert_eq!(report.applied, migrations::all().len());
    assert_eq!(report.from, 0);
    assert_eq!(report.to, migrations::latest_version());
    assert_eq!(
        schema_version(&connection).expect("version"),
        migrations::latest_version()
    );
}

#[test]
fn head_records_every_migration_that_was_applied() {
    let connection = migrated();
    let recorded: Vec<u32> = connection
        .prepare("SELECT version FROM schema_migrations ORDER BY version")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");

    let expected: Vec<u32> = migrations::all().iter().map(|m| m.version).collect();
    assert_eq!(recorded, expected);
}

#[test]
fn user_version_pragma_tracks_the_schema_version() {
    let connection = migrated();
    let user_version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("user_version");
    assert_eq!(user_version, migrations::latest_version());
}

// ---------------------------------------------------------------------------
// Acceptance: re-running is a no-op
// ---------------------------------------------------------------------------

#[test]
fn rerunning_the_migrations_is_a_no_op() {
    let mut connection = empty();
    migrate(&mut connection).expect("first migrate");
    let schema_before = schema_snapshot(&connection);

    let report = migrate(&mut connection).expect("second migrate");

    assert_eq!(report.applied, 0, "nothing is applied the second time");
    assert_eq!(report.from, migrations::latest_version());
    assert_eq!(report.to, migrations::latest_version());
    assert_eq!(
        schema_snapshot(&connection),
        schema_before,
        "the schema is byte-identical after a second run"
    );
}

#[test]
fn rerunning_the_migrations_preserves_data() {
    let mut connection = migrated();
    insert_account(&connection, "someone@example.com");

    migrate(&mut connection).expect("second migrate");

    let count: i64 = connection
        .query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))
        .expect("count accounts");
    assert_eq!(count, 1, "re-running must not touch existing rows");
}

fn schema_snapshot(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare(
            "SELECT type || ' ' || name || ' ' || coalesce(sql, '')
             FROM sqlite_master ORDER BY type, name",
        )
        .expect("prepare snapshot");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("snapshot")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect snapshot")
}

// ---------------------------------------------------------------------------
// Acceptance: every migration applies cleanly, in order
// ---------------------------------------------------------------------------

#[test]
fn migration_versions_are_sequential_from_one() {
    let versions: Vec<u32> = migrations::all().iter().map(|m| m.version).collect();
    let expected: Vec<u32> = (1..=versions.len() as u32).collect();
    assert_eq!(
        versions, expected,
        "migrations are numbered 1..n with no gaps and no reordering"
    );
}

#[test]
fn every_migration_applies_cleanly_in_order() {
    let mut connection = empty();

    for migration in migrations::all() {
        let report = migrations::migrate_with(
            &mut connection,
            &migrations::all()[..migration.version as usize],
        )
        .unwrap_or_else(|error| {
            panic!(
                "migration {} ({}) failed to apply: {error}",
                migration.version, migration.name
            )
        });

        assert_eq!(report.to, migration.version);
        assert_eq!(
            schema_version(&connection).expect("version"),
            migration.version,
            "after migration {} the schema version is {}",
            migration.version,
            migration.version
        );
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .expect("integrity check");
        assert_eq!(
            integrity, "ok",
            "migration {} left the database inconsistent",
            migration.version
        );
    }
}

#[test]
fn a_failing_migration_rolls_back_completely() {
    let mut connection = empty();
    let broken = [
        migrations::all()[0].clone(),
        Migration {
            version: 2,
            name: "broken",
            sql: "CREATE TABLE ok_so_far (id INTEGER PRIMARY KEY);
                  CREATE TABLE oops (this is not sql);",
        },
    ];

    let error = migrations::migrate_with(&mut connection, &broken).expect_err("must fail");
    assert!(
        matches!(&error, Error::Migration { version: 2, .. }),
        "expected a migration error naming version 2, got {error:?}"
    );

    assert_eq!(
        schema_version(&connection).expect("version"),
        1,
        "the database stays at the last migration that committed"
    );
    assert!(
        !table_names(&connection)
            .iter()
            .any(|name| name == "ok_so_far"),
        "no statement from the failed migration is left behind"
    );
}

#[test]
fn a_database_newer_than_this_build_is_refused() {
    let mut connection = migrated();
    // Pretend a future build wrote a migration this one has never heard of.
    let future = migrations::latest_version() + 1;
    connection
        .execute(
            "INSERT INTO schema_migrations (version, name, checksum, applied_at)
             VALUES (?1, 'from the future', 'x', 0)",
            [future],
        )
        .expect("insert future migration");

    let error = migrate(&mut connection).expect_err("must refuse to open");
    assert!(
        matches!(error, Error::SchemaTooNew { .. }),
        "expected SchemaTooNew, got {error:?}"
    );
}

#[test]
fn editing_an_already_applied_migration_is_refused() {
    let mut connection = empty();
    migrations::migrate_with(&mut connection, &migrations::all()[..1]).expect("apply first");

    let tampered = [Migration {
        version: 1,
        name: migrations::all()[0].name,
        sql: "CREATE TABLE rewritten_history (id INTEGER PRIMARY KEY);",
    }];
    let error = migrations::migrate_with(&mut connection, &tampered).expect_err("must fail");
    assert!(
        matches!(error, Error::MigrationChanged { version: 1, .. }),
        "migrations are forward-only: an applied one may never be edited, got {error:?}"
    );
}

#[test]
fn migrating_an_out_of_order_list_is_refused() {
    let mut connection = empty();
    let out_of_order = [
        Migration {
            version: 2,
            name: "second",
            sql: "CREATE TABLE a (id INTEGER PRIMARY KEY);",
        },
        Migration {
            version: 1,
            name: "first",
            sql: "CREATE TABLE b (id INTEGER PRIMARY KEY);",
        },
    ];
    let error = migrations::migrate_with(&mut connection, &out_of_order).expect_err("must fail");
    assert!(
        matches!(error, Error::MigrationOrder { .. }),
        "expected MigrationOrder, got {error:?}"
    );
}

// ---------------------------------------------------------------------------
// The schema itself
// ---------------------------------------------------------------------------

#[test]
fn head_schema_has_every_table_the_spec_requires() {
    let connection = migrated();
    let tables = table_names(&connection);

    // spec.md §6.
    for required in [
        "accounts",
        "identities",
        "mailboxes",
        "messages",
        "threads",
        "recipients",
        "attachments",
        "labels",
        "message_labels",
        "drafts",
        "sync_state",
        "settings",
        "operation_queue",
    ] {
        assert!(
            tables.iter().any(|name| name == required),
            "spec.md §6 requires a `{required}` table; have {tables:?}"
        );
    }
}

#[test]
fn head_schema_tracks_its_own_version() {
    let connection = migrated();
    assert!(
        table_names(&connection)
            .iter()
            .any(|name| name == "schema_migrations")
    );
}

#[test]
fn message_bodies_and_attachment_bytes_are_not_stored_in_sqlite() {
    let connection = migrated();
    for table in ["messages", "attachments"] {
        let mut statement = connection
            .prepare(&format!(
                "SELECT name, type FROM pragma_table_info('{table}')"
            ))
            .expect("prepare table_info");
        let columns: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("table info")
            .collect::<Result<_, _>>()
            .expect("collect");

        for (name, kind) in &columns {
            assert_ne!(
                kind.to_ascii_uppercase(),
                "BLOB",
                "{table}.{name} is a BLOB; bytes belong in the content-addressed store"
            );
        }
        assert!(
            columns.iter().any(|(name, _)| name.ends_with("blob_id")),
            "{table} must reference the blob store by key"
        );
    }
}

// ---------------------------------------------------------------------------
// Indexes that the performance budget depends on (CLAUDE.md: <16ms
// interaction, <100ms search).
// ---------------------------------------------------------------------------

#[test]
fn the_message_list_query_uses_an_index_and_never_sorts() {
    let connection = migrated();
    let plan = query_plan(
        &connection,
        "SELECT id FROM messages
         WHERE mailbox_id = 1 AND deleted_locally = 0
         ORDER BY received_at DESC, id DESC
         LIMIT 50",
    );
    assert!(
        uses_index(&plan),
        "the message-list query must use an index, plan was:\n{plan}"
    );
    assert!(
        !plan.contains("TEMP B-TREE"),
        "the message-list query must not sort; windowed paging depends on it. Plan:\n{plan}"
    );
    assert!(
        !plan.contains("SCAN messages"),
        "the message-list query must not scan the table. Plan:\n{plan}"
    );
}

#[test]
fn the_thread_list_query_uses_an_index_and_never_sorts() {
    let connection = migrated();
    let plan = query_plan(
        &connection,
        "SELECT id FROM threads WHERE account_id = 1 ORDER BY last_at DESC, id DESC LIMIT 50",
    );
    assert!(uses_index(&plan), "plan was:\n{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "plan was:\n{plan}");
}

#[test]
fn looking_up_a_thread_members_uses_an_index() {
    let connection = migrated();
    let plan = query_plan(
        &connection,
        "SELECT id FROM messages WHERE thread_id = 1 ORDER BY received_at, id",
    );
    assert!(uses_index(&plan), "plan was:\n{plan}");
    assert!(!plan.contains("TEMP B-TREE"), "plan was:\n{plan}");
}

#[test]
fn looking_up_a_message_by_rfc_message_id_uses_an_index() {
    let connection = migrated();
    let plan = query_plan(
        &connection,
        "SELECT id FROM messages WHERE account_id = 1 AND rfc_message_id = '<a@b>'",
    );
    assert!(
        uses_index(&plan),
        "JWZ threading resolves parents by Message-ID; it must be indexed. Plan:\n{plan}"
    );
    assert!(!plan.contains("SCAN messages"), "plan was:\n{plan}");
}

#[test]
fn looking_up_a_message_by_uid_uses_an_index() {
    let connection = migrated();
    let plan = query_plan(
        &connection,
        "SELECT id FROM messages WHERE mailbox_id = 1 AND uid_validity = 1 AND uid = 42",
    );
    assert!(uses_index(&plan), "plan was:\n{plan}");
}

// ---------------------------------------------------------------------------
// Constraints that keep the store honest
// ---------------------------------------------------------------------------

fn insert_account(connection: &Connection, address: &str) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts (
                 display_name, address, incoming_host, incoming_port, incoming_security,
                 incoming_username, outgoing_host, outgoing_port, outgoing_security,
                 outgoing_username, auth_method, enabled, created_at)
             VALUES (?1, ?1, 'imap.mail.me.com', 993, 'tls', ?1,
                     'smtp.mail.me.com', 587, 'starttls', ?1, 'app_password', 1, 0)",
            [address],
        )
        .expect("insert account");
    connection.last_insert_rowid()
}

fn insert_mailbox(connection: &Connection, account_id: i64, path: &str) -> i64 {
    connection
        .execute(
            "INSERT INTO mailboxes (account_id, name, path, role) VALUES (?1, ?2, ?2, 'inbox')",
            rusqlite::params![account_id, path],
        )
        .expect("insert mailbox");
    connection.last_insert_rowid()
}

#[test]
fn foreign_keys_are_declared_and_enforced() {
    let connection = migrated();
    let error = connection
        .execute(
            "INSERT INTO mailboxes (account_id, name, path) VALUES (9999, 'x', 'x')",
            [],
        )
        .expect_err("an orphan mailbox must be rejected");
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("foreign key"),
        "expected a foreign key violation, got {error}"
    );
}

#[test]
fn deleting_an_account_cascades_to_its_mailboxes_and_messages() {
    let connection = migrated();
    let account = insert_account(&connection, "cascade@example.com");
    let mailbox = insert_mailbox(&connection, account, "INBOX");
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at) VALUES (?1, ?2, 0)",
            rusqlite::params![account, mailbox],
        )
        .expect("insert message");

    connection
        .execute("DELETE FROM accounts WHERE id = ?1", [account])
        .expect("delete account");

    for table in ["mailboxes", "messages"] {
        let count: i64 = connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 0, "{table} should have cascaded away");
    }
}

#[test]
fn a_mailbox_path_is_unique_within_an_account() {
    let connection = migrated();
    let account = insert_account(&connection, "unique@example.com");
    insert_mailbox(&connection, account, "INBOX");
    let error = connection
        .execute(
            "INSERT INTO mailboxes (account_id, name, path) VALUES (?1, 'INBOX', 'INBOX')",
            [account],
        )
        .expect_err("duplicate path must be rejected");
    assert!(error.to_string().to_ascii_lowercase().contains("unique"));
}

#[test]
fn a_label_name_is_unique_per_account_case_insensitively() {
    let connection = migrated();
    let account = insert_account(&connection, "labels@example.com");
    connection
        .execute(
            "INSERT INTO labels (account_id, name) VALUES (?1, 'Work')",
            [account],
        )
        .expect("insert label");
    let error = connection
        .execute(
            "INSERT INTO labels (account_id, name) VALUES (?1, 'work')",
            [account],
        )
        .expect_err("labels are unique case-insensitively per account");
    assert!(error.to_string().to_ascii_lowercase().contains("unique"));
}

#[test]
fn an_account_has_at_most_one_default_identity() {
    let connection = migrated();
    let account = insert_account(&connection, "ident@example.com");
    let insert_default = "INSERT INTO identities (account_id, display_name, address, is_default)
                          VALUES (?1, ?2, ?2, 1)";
    connection
        .execute(insert_default, rusqlite::params![account, "a@example.com"])
        .expect("the first default identity");
    let error = connection
        .execute(insert_default, rusqlite::params![account, "b@example.com"])
        .expect_err("a second default identity must be rejected");
    assert!(error.to_string().to_ascii_lowercase().contains("unique"));

    // A non-default identity alongside it is fine.
    connection
        .execute(
            "INSERT INTO identities (account_id, display_name, address, is_default)
             VALUES (?1, 'b@example.com', 'b@example.com', 0)",
            [account],
        )
        .expect("a second non-default identity");

    let defaults: i64 = connection
        .query_row(
            "SELECT count(*) FROM identities WHERE is_default = 1",
            [],
            |row| row.get(0),
        )
        .expect("count defaults");
    assert_eq!(defaults, 1);
}

#[test]
fn a_uid_is_unique_within_a_mailbox_generation() {
    let connection = migrated();
    let account = insert_account(&connection, "uid@example.com");
    let mailbox = insert_mailbox(&connection, account, "INBOX");
    let insert = "INSERT INTO messages (account_id, mailbox_id, received_at, uid, uid_validity)
                  VALUES (?1, ?2, 0, 7, 100)";
    connection
        .execute(insert, rusqlite::params![account, mailbox])
        .expect("first insert");
    let error = connection
        .execute(insert, rusqlite::params![account, mailbox])
        .expect_err("the same UID under the same UIDVALIDITY is one message");
    assert!(error.to_string().to_ascii_lowercase().contains("unique"));

    // A new UIDVALIDITY generation reuses the UID space; that must be allowed.
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, uid, uid_validity)
             VALUES (?1, ?2, 0, 7, 101)",
            rusqlite::params![account, mailbox],
        )
        .expect("a new generation may reuse the UID");

    // Locally composed messages have no UID at all, and there may be many.
    for _ in 0..2 {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at) VALUES (?1, ?2, 0)",
                rusqlite::params![account, mailbox],
            )
            .expect("messages without a UID are not constrained");
    }
}

#[test]
fn settings_are_unique_per_scope() {
    let connection = migrated();
    let account = insert_account(&connection, "settings@example.com");

    connection
        .execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('sidebar.width', '280', 0)",
            [],
        )
        .expect("global setting");
    connection
        .execute(
            "INSERT INTO settings (key, account_id, value, updated_at)
             VALUES ('sidebar.width', ?1, '320', 0)",
            [account],
        )
        .expect("a per-account setting shadows the global one");

    let error = connection
        .execute(
            "INSERT INTO settings (key, value, updated_at) VALUES ('sidebar.width', '300', 0)",
            [],
        )
        .expect_err("a global key may only appear once");
    assert!(error.to_string().to_ascii_lowercase().contains("unique"));
}

// ---------------------------------------------------------------------------
// sync_state: the sync engine updates it atomically with the writes it describes
// ---------------------------------------------------------------------------

#[test]
fn sync_state_holds_uidvalidity_uidnext_and_highestmodseq_per_mailbox() {
    let connection = migrated();
    let columns: Vec<String> = connection
        .prepare("SELECT name FROM pragma_table_info('sync_state')")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");

    for required in ["mailbox_id", "uid_validity", "uid_next", "highest_mod_seq"] {
        assert!(
            columns.iter().any(|name| name == required),
            "sync_state needs `{required}`; have {columns:?}"
        );
    }
}

#[test]
fn sync_state_and_message_writes_commit_in_one_transaction() {
    let mut connection = migrated();
    let account = insert_account(&connection, "sync@example.com");
    let mailbox = insert_mailbox(&connection, account, "INBOX");

    // A mailbox that has never been synced has no state at all.
    let state: Option<i64> = connection
        .query_row(
            "SELECT uid_validity FROM sync_state WHERE mailbox_id = ?1",
            [mailbox],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    assert!(state.is_none(), "'never synced' is representable");

    let transaction = connection.transaction().expect("begin");
    transaction
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, uid, uid_validity)
             VALUES (?1, ?2, 0, 1, 100)",
            rusqlite::params![account, mailbox],
        )
        .expect("write message");
    transaction
        .execute(
            "INSERT INTO sync_state (mailbox_id, account_id, uid_validity, uid_next, highest_mod_seq)
             VALUES (?1, ?2, 100, 2, 55)",
            rusqlite::params![mailbox, account],
        )
        .expect("write sync state");
    transaction.rollback().expect("rollback");

    let messages: i64 = connection
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("count messages");
    let states: i64 = connection
        .query_row("SELECT count(*) FROM sync_state", [], |row| row.get(0))
        .expect("count sync state");
    assert_eq!(
        (messages, states),
        (0, 0),
        "a crash mid-sync must leave neither the messages nor the state it described"
    );
}

// ---------------------------------------------------------------------------
// operation_queue: local-first mutations survive a restart, in order
// ---------------------------------------------------------------------------

#[test]
fn the_operation_queue_preserves_enqueue_order_and_carries_an_inverse() {
    let connection = migrated();
    let account = insert_account(&connection, "queue@example.com");

    for op in ["set_flag", "move", "delete"] {
        connection
            .execute(
                "INSERT INTO operation_queue
                     (account_id, op_type, target_kind, target_id, payload, inverse,
                      created_at, updated_at)
                 VALUES (?1, ?2, 'message', 1, '{}', '{}', 0, 0)",
                rusqlite::params![account, op],
            )
            .expect("enqueue");
    }

    let order: Vec<String> = connection
        .prepare("SELECT op_type FROM operation_queue ORDER BY id")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect");
    assert_eq!(order, ["set_flag", "move", "delete"]);
}
