//! `UID FETCH` of headers: `SELECT` caching, `ENVELOPE`/`BODYSTRUCTURE`
//! mapping, `CHANGEDSINCE`, and cancellation. All replayed with no socket —
//! see `imap_mailboxes.rs` for the same pattern applied to `LIST`.

use std::sync::Arc;

use postio_imap::backend::UidSet;
use postio_imap::cancel::CancelToken;
use postio_imap::imap::{
    ConnectionPool, ConnectionSettings, IMAPS_PORT, ImapScript, PoolConfig, Priority,
    ScriptedConnector, fetch_headers,
};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::{ModSeq, TransportSecurity, Uid};

const ACCOUNT: &str = "someone@example.com";

async fn pool_over(connector: ScriptedConnector) -> ConnectionPool {
    let store = MemorySecretStore::new();
    let key = AccountKey::new(ACCOUNT);
    store
        .store(&key, &Password::new("app-specific-password"))
        .await
        .expect("seed the keyring");

    ConnectionPool::new(
        ConnectionSettings::new(
            "imap.example.com",
            IMAPS_PORT,
            TransportSecurity::Tls,
            ACCOUNT,
        ),
        key,
        Arc::new(store),
        Arc::new(connector),
        PoolConfig::default(),
    )
}

/// One `FETCH` response line for a message with a simple text body and one
/// `References` entry recovered from the extra header fetch.
fn fetch_line(seq: u32, uid: u32) -> String {
    format!(
        "* {seq} FETCH (UID {uid} FLAGS (\\Seen) \
         INTERNALDATE \"07-Feb-1994 21:52:25 -0800\" RFC822.SIZE 158 \
         ENVELOPE (\"Mon, 7 Feb 1994 21:52:25 -0800\" \"Design review notes\" \
         ((\"Ada Lovelace\" NIL \"ada\" \"example.com\")) NIL NIL \
         ((\"Grace Hopper\" NIL \"grace\" \"example.com\")) NIL NIL NIL \
         \"<msg-{uid}@example.com>\") \
         BODY[HEADER.FIELDS (REFERENCES)] \"References: <msg-0@example.com>\" \
         BODYSTRUCTURE (\"TEXT\" \"PLAIN\" (\"CHARSET\" \"us-ascii\") NIL NIL \"7BIT\" 158 4 NIL NIL NIL NIL) \
         MODSEQ (400{uid}))"
    )
}

fn select_reply() -> String {
    "* 5 EXISTS\n* 0 RECENT\n* OK [UIDVALIDITY 100] UIDs valid\n{tag} OK SELECT completed"
        .to_owned()
}

fn fetch_reply(lines: &[String]) -> String {
    format!("{}\n{{tag}} OK FETCH completed", lines.join("\n"))
}

fn script() -> ImapScript {
    ImapScript::extensions_hidden_until_login()
        .on("SELECT", select_reply().as_str())
        .on("FETCH", fetch_reply(&[fetch_line(1, 101)]).as_str())
}

#[tokio::test]
async fn fetching_headers_maps_envelope_flags_size_and_bodystructure() {
    let pool = pool_over(ScriptedConnector::new(script())).await;
    let uids = UidSet::single(Uid::new(101));

    let messages = fetch_headers(
        &pool,
        "INBOX",
        &uids,
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(messages.len(), 1);
    let message = &messages[0];
    assert_eq!(message.uid, Uid::new(101));
    assert_eq!(message.uid_validity.get(), 100);
    assert_eq!(message.size, 158);
    assert_eq!(message.mod_seq, Some(ModSeq::new(400_101)));

    let envelope = message.envelope.as_ref().expect("an envelope");
    assert_eq!(envelope.subject.as_deref(), Some("Design review notes"));
    assert_eq!(envelope.from[0].address, "ada@example.com");
    assert_eq!(envelope.from[0].name.as_deref(), Some("Ada Lovelace"));
    assert_eq!(envelope.to[0].address, "grace@example.com");
    assert_eq!(
        envelope
            .message_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        Some("<msg-101@example.com>".to_owned())
    );
    assert_eq!(
        envelope
            .references
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>(),
        vec!["<msg-0@example.com>"]
    );

    let structure = message.structure.as_ref().expect("a body structure");
    let text = structure.text_part().expect("a text part");
    assert_eq!(text.section(), "1");
    assert_eq!(text.mime_type(), "text/plain");
}

#[tokio::test]
async fn a_second_fetch_of_the_same_mailbox_does_not_reissue_select() {
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply().as_str())
            .on("FETCH", fetch_reply(&[fetch_line(1, 101)]).as_str()),
    );
    let pool = pool_over(connector.clone()).await;
    let uids = UidSet::single(Uid::new(101));

    fetch_headers(
        &pool,
        "INBOX",
        &uids,
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();
    fetch_headers(
        &pool,
        "INBOX",
        &uids,
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    let selects = connector
        .log()
        .commands()
        .iter()
        .filter(|command| command.contains("SELECT"))
        .count();
    assert_eq!(selects, 1, "the cached selection should not be reissued");
}

#[tokio::test]
async fn switching_mailboxes_reselects() {
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply().as_str())
            .on("FETCH", fetch_reply(&[fetch_line(1, 101)]).as_str()),
    );
    let pool = pool_over(connector.clone()).await;
    let uids = UidSet::single(Uid::new(101));

    fetch_headers(
        &pool,
        "INBOX",
        &uids,
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();
    fetch_headers(
        &pool,
        "Archive",
        &uids,
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    let selects = connector
        .log()
        .commands()
        .iter()
        .filter(|command| command.contains("SELECT"))
        .count();
    assert_eq!(selects, 2, "a different mailbox must be reselected");
}

#[tokio::test]
async fn changed_since_selects_with_condstore_and_sends_the_modifier() {
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply().as_str())
            .on("FETCH", fetch_reply(&[fetch_line(1, 101)]).as_str()),
    );
    let pool = pool_over(connector.clone()).await;
    let uids = UidSet::single(Uid::new(101));

    fetch_headers(
        &pool,
        "INBOX",
        &uids,
        Some(ModSeq::new(3000)),
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    let commands = connector.log().commands();
    assert!(
        commands
            .iter()
            .any(|command| command.contains("SELECT") && command.contains("CONDSTORE")),
        "expected a CONDSTORE select, got: {commands:?}"
    );
    assert!(
        commands
            .iter()
            .any(|command| command.contains("FETCH") && command.contains("CHANGEDSINCE 3000")),
        "expected a CHANGEDSINCE fetch modifier, got: {commands:?}"
    );
}

#[tokio::test]
async fn an_empty_uid_set_never_opens_a_connection() {
    let connector = ScriptedConnector::new(script());
    let pool = pool_over(connector.clone()).await;

    let messages = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::new(),
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    assert!(messages.is_empty());
    assert!(connector.log().tls.is_empty());
}

#[tokio::test]
async fn a_cancelled_token_stops_before_any_round_trip() {
    let connector = ScriptedConnector::new(script());
    let pool = pool_over(connector.clone()).await;
    let cancel = CancelToken::new();
    cancel.cancel();

    let error = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::single(Uid::new(101)),
        None,
        Priority::Interactive,
        &cancel,
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        postio_imap::backend::BackendError::Cancelled
    ));
    assert!(connector.log().tls.is_empty());
}
