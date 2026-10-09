//! The content-ownership step (#1780, ADR 0046) carries each existing
//! message's bytes and flags across, and infers no sharing from them.
//!
//! `migrations.rs` proves the step leaves a store shaped as a fresh one; this
//! proves what was in it survives, and that two messages naming the same RFC
//! Message-ID still own content of their own.
use postio_model::MessageId;
use postio_storage::{Store, repository::MessageRepository, schema, test_support};

#[tokio::test]
async fn content_split_preserves_existing_mail_and_does_not_infer_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.db");
    let key = test_support::key();
    {
        let database = turso::Builder::new_local(path.to_str().unwrap())
            .experimental_encryption(true)
            .with_encryption(turso::EncryptionOpts {
                cipher: "aes256gcm".into(),
                hexkey: key.to_hex().to_string(),
            })
            .experimental_triggers(true)
            .experimental_index_method(true)
            .experimental_generated_columns(true)
            .build()
            .await
            .unwrap();
        let connection = database.connect().unwrap();
        connection
            .execute_batch(include_str!("../schemas/815185a3.sql"))
            .await
            .unwrap();
        connection.execute_batch("INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port, incoming_security, incoming_username, outgoing_host, outgoing_port, outgoing_security, outgoing_username, auth_method, enabled, created_at) VALUES (1,'Test','test@example.test','imap.example.test',993,'tls','test','smtp.example.test',465,'tls','test','password',1,0);
            INSERT INTO mailboxes (id, account_id, name, path, role) VALUES (1,1,'INBOX','INBOX','inbox');
            INSERT INTO messages (id,account_id,mailbox_id,received_at,sort_at,rfc_message_id,body_text,body_state,remote_id,uid,uid_validity,seen) VALUES (10,1,1,0,0,'<shared@example.test>','retained bytes','full','9:1',1,9,1),(20,1,1,1,1,'<shared@example.test>','different bytes','full','9:2',2,9,0);").await.unwrap();
        connection
            .execute("UPDATE messages SET flags = ?1 WHERE id = 10", ("\\Seen",))
            .await
            .unwrap();
        // The stamp that build wrote: what the file is named for.
        connection
            .execute(
                &format!(
                    "PRAGMA user_version = {}",
                    schema::fingerprint(include_str!("../schemas/815185a3.sql"))
                ),
                (),
            )
            .await
            .unwrap();
    }
    let store = Store::open(&path, &key)
        .await
        .expect("known schema migrates");
    let connection = store.connect().await.unwrap();
    let repo = MessageRepository::new(&connection);
    for (id, text, seen) in [(10, "retained bytes", true), (20, "different bytes", false)] {
        let message = repo.get(MessageId::new(id)).await.unwrap().unwrap();
        assert_eq!(message.server.content_identity, None);
        assert_eq!(message.flags.is_seen(), seen);
        assert_eq!(
            repo.body(message.id)
                .await
                .unwrap()
                .unwrap()
                .text
                .as_deref(),
            Some(text)
        );
    }
    let contents: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM message_contents", ())
            .await
            .unwrap();
    assert_eq!(contents, 2, "no RFC identity guessing during migration");
    drop(connection);
    drop(store);
    Store::open(&path, &key)
        .await
        .expect("migrated store reopens");
}
