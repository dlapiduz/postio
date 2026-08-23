//! `UID FETCH` of headers: `SELECT` caching, `ENVELOPE`/`BODYSTRUCTURE`
//! mapping, `CHANGEDSINCE`, and cancellation. All replayed with no socket —
//! see `imap_mailboxes.rs` for the same pattern applied to `LIST`.

use std::sync::Arc;

use postio_imap::backend::UidSet;
use postio_imap::cancel::CancelToken;
use postio_imap::imap::{
    ConnectionPool, ConnectionSettings, IMAPS_PORT, ImapScript, PoolConfig, Priority,
    RustlsConnector, ScriptedConnector, fetch_headers,
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

// ---------------------------------------------------------------------------
// Resync integrity: a line io-imap could not decode must not pass as success
// ---------------------------------------------------------------------------

/// A `-1` sequence number is not a valid IMAP `seq-number` (`1*DIGIT`, never
/// signed) — `imap-types` models one as `NonZeroU32` regardless — so
/// `io-imap` cannot decode this line and, per ADR 0001, silently drops it
/// rather than failing the command. Apple Developer Forums thread 694251
/// reports exactly this shape from iCloud under QRESYNC.
const UNDECODABLE_UNTAGGED_LINE: &str = "* -1 FETCH (FLAGS (\\Seen))";

#[tokio::test]
async fn a_line_io_imap_could_not_decode_forces_a_full_resync_not_a_silent_success() {
    let reply = fetch_reply(&[UNDECODABLE_UNTAGGED_LINE.to_owned(), fetch_line(1, 101)]);
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply().as_str())
            .on("FETCH", reply.as_str()),
    );
    let pool = pool_over(connector).await;

    let error = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::single(Uid::new(101)),
        Some(ModSeq::new(3000)),
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap_err();

    let postio_imap::backend::BackendError::ResyncIntegrityLost { mailbox, skipped } = &error
    else {
        panic!("expected ResyncIntegrityLost, got {error:?}");
    };
    assert_eq!(mailbox, "INBOX");
    assert!(*skipped >= 1);
    assert!(
        error.requires_full_resync(),
        "a lost delta must be reported as needing a full resync, the same \
         predicate a UIDVALIDITY change reports through"
    );
}

#[tokio::test]
async fn the_same_undecodable_line_is_tolerated_outside_a_changedsince_fetch() {
    // The integrity check only brackets an incremental (CHANGEDSINCE)
    // fetch: it is where a silently dropped delta is indistinguishable from
    // "nothing changed," which is the failure mode this exists to catch. A
    // plain fetch with no baseline to miss deltas from is not that case.
    let reply = fetch_reply(&[UNDECODABLE_UNTAGGED_LINE.to_owned(), fetch_line(1, 101)]);
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply().as_str())
            .on("FETCH", reply.as_str()),
    );
    let pool = pool_over(connector).await;

    // This still makes io-imap skip a real, undecodable line and bump the
    // real (process-wide) skip counter — production has no reason to care
    // outside a CHANGEDSINCE fetch, but a sibling test *measuring* a delta
    // of its own, running concurrently in the same test binary, would
    // otherwise intermittently see this skip as its own.
    let _exclusive = postio_imap::imap::skip_counter_exclusive_measurement().await;

    let messages = fetch_headers(
        &pool,
        "INBOX",
        &UidSet::single(Uid::new(101)),
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(messages.len(), 1);
}

// ---------------------------------------------------------------------------
// Live server. Ignored by default; needs a real account. See
// `imap_session.rs` for the connect/capability live tests and
// `imap_mailboxes.rs` for folder discovery — this covers header fetch.
// ---------------------------------------------------------------------------

/// Reads the live-test credentials, or skips.
///
/// `POSTIO_TEST_IMAP_USER` and `POSTIO_TEST_IMAP_PASSWORD` — for a provider
/// that requires one, an app-specific password. The host comes from Postio's
/// preset table when it ships one for the address's domain, and
/// `POSTIO_TEST_IMAP_HOST` overrides it for anything else.
async fn live_pool() -> Option<ConnectionPool> {
    let user = std::env::var("POSTIO_TEST_IMAP_USER").ok()?;
    let password = std::env::var("POSTIO_TEST_IMAP_PASSWORD").ok()?;

    let store = MemorySecretStore::new();
    let key = AccountKey::new(&user);
    store
        .store(&key, &Password::new(password))
        .await
        .expect("seed the keyring");

    let settings = ConnectionSettings::preset_for(&user).unwrap_or_else(|| {
        ConnectionSettings::new(
            std::env::var("POSTIO_TEST_IMAP_HOST").expect("POSTIO_TEST_IMAP_HOST"),
            IMAPS_PORT,
            TransportSecurity::Tls,
            &user,
        )
    });

    Some(ConnectionPool::new(
        settings,
        key,
        Arc::new(store),
        Arc::new(RustlsConnector::new().expect("TLS configuration")),
        PoolConfig::default(),
    ))
}

#[tokio::test]
#[ignore = "talks to a live IMAP server; set POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD"]
async fn live_server_fetches_real_headers() {
    let Some(pool) = live_pool().await else {
        panic!("POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD must be set");
    };

    // A bounded prefix, not "1:*": the point is proving the wire path works
    // against a real server, not paging through everything a real inbox
    // holds. Read-only (BODY.PEEK carries no \Seen side effect), so there is
    // nothing to clean up afterward.
    let uids = UidSet::range(Uid::new(1), Uid::new(500));

    let messages = fetch_headers(
        &pool,
        "INBOX",
        &uids,
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .expect("live header fetch");

    println!("fetched {} header(s) from INBOX", messages.len());
    for message in &messages {
        assert!(
            uids.contains(message.uid),
            "{:?} was outside the requested range",
            message.uid
        );
        assert!(
            message.envelope.is_some(),
            "a header fetch must carry ENVELOPE"
        );
        assert!(
            message.structure.is_some(),
            "a header fetch must carry BODYSTRUCTURE"
        );
    }

    pool.close();
}
