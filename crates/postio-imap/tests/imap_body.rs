//! Streaming a message body or MIME part: bounded-window partial fetch,
//! section addressing, missing messages and cancellation. All replayed with
//! no socket — see `imap_mailboxes.rs` for the same pattern applied to
//! `LIST`.

use std::sync::Arc;

use postio_imap::backend::{BackendError, BodyPart, CountingSink, UidSet, VecSink};
use postio_imap::cancel::CancelToken;
use postio_imap::imap::{
    ConnectionPool, ConnectionSettings, IMAPS_PORT, ImapScript, PARTIAL_FETCH_WINDOW, PoolConfig,
    Priority, RustlsConnector, ScriptedConnector, fetch_headers, fetch_part,
};
use postio_imap::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_model::{TransportSecurity, Uid};

const ACCOUNT: &str = "someone@example.com";
const WINDOW: usize = PARTIAL_FETCH_WINDOW as usize;

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

fn select_reply() -> &'static str {
    "* 5 EXISTS\n* 0 RECENT\n* OK [UIDVALIDITY 100] UIDs valid\n{tag} OK SELECT completed"
}

/// A `* 1 FETCH` reply carrying one literal window of `payload` at `offset`.
fn window_reply(section: &str, offset: usize, payload: &str) -> String {
    format!(
        "* 1 FETCH (BODY[{section}]<{offset}> {{{len}}}\r\n{payload})\n{{tag}} OK FETCH completed",
        len = payload.len()
    )
}

fn ascii_payload(len: usize) -> String {
    (0..len).map(|i| (b'A' + (i % 26) as u8) as char).collect()
}

#[tokio::test]
async fn a_large_attachment_streams_in_bounded_windows_not_one_buffer() {
    let full = ascii_payload(WINDOW);
    let tail = ascii_payload(500);

    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply())
            .on(format!("<0.{WINDOW}>"), window_reply("2", 0, &full))
            .on(
                format!("<{WINDOW}.{WINDOW}>"),
                window_reply("2", WINDOW, &full),
            )
            .on(
                format!("<{}.{WINDOW}>", WINDOW * 2),
                window_reply("2", WINDOW * 2, &tail),
            ),
    );
    let pool = pool_over(connector).await;
    let mut sink = VecSink::new();

    let result = fetch_part(
        &pool,
        "INBOX",
        Uid::new(101),
        &BodyPart::section("2"),
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(result.uid, Uid::new(101));
    assert_eq!(result.bytes_written, (WINDOW * 2 + 500) as u64);
    assert_eq!(sink.len(), WINDOW * 2 + 500);
    assert_eq!(
        sink.chunks(),
        3,
        "one chunk per round trip, not one big one"
    );
    assert!(sink.is_finished());
    assert_eq!(&sink.as_slice()[..WINDOW], full.as_bytes());
    assert_eq!(&sink.as_slice()[WINDOW * 2..], tail.as_bytes());
}

#[tokio::test]
async fn a_part_smaller_than_one_window_finishes_in_one_round_trip() {
    let payload = ascii_payload(37);
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply())
            .on("FETCH", window_reply("1", 0, &payload)),
    );
    let pool = pool_over(connector.clone()).await;
    let mut sink = VecSink::new();

    fetch_part(
        &pool,
        "INBOX",
        Uid::new(101),
        &BodyPart::Whole,
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(sink.into_inner(), payload.into_bytes());
    assert_eq!(
        connector
            .log()
            .commands()
            .iter()
            .filter(|command| command.contains("FETCH"))
            .count(),
        1
    );
}

#[tokio::test]
async fn a_whole_message_larger_than_one_window_still_costs_one_round_trip() {
    // BodyPart::Whole drives io-imap's real streaming FETCH coroutine
    // (BODY.PEEK[]) instead of the windowed partial-fetch loop the other
    // BodyPart variants use, so a message several windows large must still
    // be exactly one UID FETCH — proving the windowing loop was bypassed,
    // not just that the bytes eventually arrived.
    let len = u32::try_from(WINDOW * 3 + 17).unwrap();
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply())
            .on_generated(
                "FETCH",
                "* 1 FETCH (BODY[] {",
                len,
                ")\n{tag} OK FETCH completed",
            ),
    );
    let pool = pool_over(connector.clone()).await;
    let mut sink = VecSink::new();

    let result = fetch_part(
        &pool,
        "INBOX",
        Uid::new(101),
        &BodyPart::Whole,
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    assert_eq!(result.bytes_written, u64::from(len));
    assert_eq!(sink.len(), len as usize);
    assert!(sink.is_finished());
    assert_eq!(
        connector
            .log()
            .commands()
            .iter()
            .filter(|command| command.contains("FETCH"))
            .count(),
        1,
        "a whole-message fetch must cost exactly one round trip regardless of size"
    );
}

#[tokio::test]
async fn headers_and_text_ask_for_their_own_named_sections() {
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply())
            .on("FETCH", window_reply("HEADER", 0, "From: a@example.com")),
    );
    let pool = pool_over(connector.clone()).await;
    let mut sink = VecSink::new();

    fetch_part(
        &pool,
        "INBOX",
        Uid::new(101),
        &BodyPart::Headers,
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap();

    let commands = connector.log().commands();
    assert!(
        commands
            .iter()
            .any(|command| command.contains("BODY.PEEK[HEADER]")),
        "expected a HEADER section fetch, got: {commands:?}"
    );
}

#[tokio::test]
async fn a_message_absent_from_the_response_is_no_such_message() {
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login()
            .on("SELECT", select_reply())
            .on("FETCH", "{tag} OK FETCH completed"),
    );
    let pool = pool_over(connector).await;
    let mut sink = VecSink::new();

    let error = fetch_part(
        &pool,
        "INBOX",
        Uid::new(404),
        &BodyPart::Whole,
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap_err();

    assert!(matches!(error, BackendError::NoSuchMessage { .. }));
    assert!(sink.is_empty());
    assert!(!sink.is_finished());
}

#[tokio::test]
async fn a_cancelled_token_stops_before_any_round_trip() {
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login().on("SELECT", select_reply()),
    );
    let pool = pool_over(connector.clone()).await;
    let cancel = CancelToken::new();
    cancel.cancel();
    let mut sink = VecSink::new();

    let error = fetch_part(
        &pool,
        "INBOX",
        Uid::new(101),
        &BodyPart::Whole,
        &mut sink,
        Priority::Interactive,
        &cancel,
    )
    .await
    .unwrap_err();

    assert!(matches!(error, BackendError::Cancelled));
    assert!(connector.log().tls.is_empty());
    assert!(!sink.is_finished());
}

#[tokio::test]
async fn a_malformed_section_number_never_reaches_the_wire() {
    let connector = ScriptedConnector::new(
        ImapScript::extensions_hidden_until_login().on("SELECT", select_reply()),
    );
    let pool = pool_over(connector.clone()).await;
    let mut sink = VecSink::new();

    let error = fetch_part(
        &pool,
        "INBOX",
        Uid::new(101),
        &BodyPart::section("2.x"),
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("2.x"));
    assert!(connector.log().tls.is_empty());
}

// ---------------------------------------------------------------------------
// Live server. Ignored by default; needs a real account. See
// `imap_session.rs` for the connect/capability live tests and
// `imap_mailboxes.rs` for folder discovery — this covers body fetch.
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
async fn live_server_streams_a_real_body() {
    let Some(pool) = live_pool().await else {
        panic!("POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD must be set");
    };

    // Discover a real UID to fetch: a bounded header prefix first, same as
    // `imap_fetch.rs`'s live test, rather than assuming one.
    let uids = UidSet::range(Uid::new(1), Uid::new(500));
    let headers = fetch_headers(
        &pool,
        "INBOX",
        &uids,
        None,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .expect("live header fetch");

    let Some(message) = headers.first() else {
        println!("INBOX has no message in UID range 1:500; skipping the body fetch");
        pool.close();
        return;
    };

    // `CountingSink`, not `VecSink`: this only needs to prove the round trip
    // and the streaming path work against a real server, not inspect the
    // content. Read-only (BODY.PEEK carries no \Seen side effect), so there
    // is nothing to clean up afterward.
    let mut sink = CountingSink::new();
    let fetched = fetch_part(
        &pool,
        "INBOX",
        message.uid,
        &BodyPart::Whole,
        &mut sink,
        Priority::Interactive,
        &CancelToken::new(),
    )
    .await
    .expect("live body fetch");

    println!(
        "streamed {} byte(s) for UID {} from INBOX (RFC822.SIZE reported {})",
        sink.bytes(),
        message.uid,
        message.size
    );
    assert_eq!(fetched.bytes_written, sink.bytes());
    assert!(
        fetched.bytes_written > 0,
        "a real message must not be empty"
    );

    pool.close();
}
