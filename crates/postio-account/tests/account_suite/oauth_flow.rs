//! Acceptance tests for #192 — the OAuth flow: system browser, loopback
//! redirect, PKCE, real cancellation.
//!
//! Nothing here touches the real network or a real browser. The "browser"
//! is played by the test itself, completing the redirect over loopback TCP
//! exactly the way a consenting user's tab would; the authorization server
//! is an in-process mock on an ephemeral loopback port. Both are the kind
//! of local-only, port-ephemeral I/O CLAUDE.md's "no network in the
//! default suite" rule already treats as fine — the same footing
//! `tests/imap_loopback.rs` runs the real client stack on.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::thread;

use postio_account::cancel::CancelToken;
use postio_account::oauth::{AuthorizeRequest, BrowserOpener, OAuthError, authorize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use url::Url;

/// A [`BrowserOpener`] that hands the authorize URL to a channel instead of
/// opening a real desktop browser, so the test can play the browser's part
/// itself.
struct ChannelOpener(Mutex<Option<oneshot::Sender<Url>>>);

impl ChannelOpener {
    fn new() -> (Self, oneshot::Receiver<Url>) {
        let (tx, rx) = oneshot::channel();
        (Self(Mutex::new(Some(tx))), rx)
    }
}

impl BrowserOpener for ChannelOpener {
    fn open(&self, url: &Url) -> std::io::Result<()> {
        if let Some(tx) = self.0.lock().expect("channel opener mutex").take() {
            let _ = tx.send(url.clone());
        }
        Ok(())
    }
}

/// A one-shot mock token endpoint: accepts one connection, reads one
/// request, answers with a fixed JSON body, on a background thread so the
/// async flow under test can drive real (loopback) I/O against it.
struct MockTokenServer {
    url: Url,
    handle: Option<thread::JoinHandle<String>>,
}

impl MockTokenServer {
    fn start(body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock token server");
        let port = listener.local_addr().expect("addr").port();
        let url: Url = format!("http://127.0.0.1:{port}/token").parse().unwrap();

        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept the token request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut request_line = String::new();
            reader.read_line(&mut request_line).expect("request line");

            let mut content_length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).expect("header line");
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut discard = vec![0u8; content_length];
            reader.read_exact(&mut discard).expect("body");

            let mut stream = stream;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");

            request_line
        });

        Self {
            url,
            handle: Some(handle),
        }
    }

    /// Whether the server ever accepted a connection, without blocking:
    /// used to prove a token exchange did **not** fire.
    fn was_contacted(mut self) -> bool {
        match self.handle.take() {
            Some(handle) => {
                // The listener has at most one client in every test that
                // calls this, and that client (if any) has already finished
                // talking by the time the flow under test has returned —
                // so a short join is enough to tell "never connected" from
                // "connected".
                for _ in 0..50 {
                    if handle.is_finished() {
                        return true;
                    }
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                false
            }
            None => true,
        }
    }
}

fn request(token_endpoint: Url) -> AuthorizeRequest {
    AuthorizeRequest {
        client_id: "the-client".to_string(),
        client_secret: None,
        authorize_endpoint: "http://127.0.0.1:1/authorize".parse().unwrap(),
        token_endpoint,
        scopes: vec!["mail.read".to_string()],
    }
}

/// Connects to the loopback redirect as a consenting browser tab would,
/// reading `code`/`state`/`redirect_uri` straight off the URL the flow
/// handed its [`BrowserOpener`].
///
/// Async, over `tokio::net::TcpStream`, rather than a blocking
/// `std::net::TcpStream`: `#[tokio::test]` runs on a single-threaded
/// runtime, and the `authorize` task under test lives on that same thread.
/// A blocking connect/write/read here would never yield the thread back to
/// the scheduler, so `authorize`'s own `accept()` could never run —
/// deadlock, not a slow test.
async fn play_the_browser(authorize_url: &Url, code: &str) {
    let pairs: std::collections::HashMap<_, _> = authorize_url.query_pairs().into_owned().collect();
    let redirect: Url = pairs["redirect_uri"].parse().expect("redirect_uri parses");
    let state = &pairs["state"];

    let mut stream = TcpStream::connect((
        redirect.host_str().expect("loopback host"),
        redirect.port().expect("bound port"),
    ))
    .await
    .expect("connect to the loopback listener");
    stream
        .write_all(
            format!("GET /?code={code}&state={state} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("write the redirect");
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response).await;
}

#[tokio::test]
async fn the_full_flow_against_a_mock_authorization_server_yields_tokens() {
    let server = MockTokenServer::start(
        r#"{"access_token":"final-access-token","refresh_token":"final-refresh-token","expires_in":3600,"token_type":"Bearer"}"#,
    );
    let (opener, opened) = ChannelOpener::new();
    let cancel = CancelToken::new();
    let req = request(server.url.clone());

    let flow = tokio::spawn(async move { authorize(req, &opener, &cancel).await });

    let authorize_url = opened.await.expect("the flow opens a browser URL");
    play_the_browser(&authorize_url, "the-auth-code").await;

    let response = flow
        .await
        .expect("the flow task joins")
        .expect("authorize succeeds");

    assert_eq!(response.access_token.expose(), "final-access-token");
    assert_eq!(
        response.refresh_token.as_ref().map(|p| p.expose()),
        Some("final-refresh-token")
    );

    assert!(
        server.was_contacted(),
        "the token endpoint must have been reached"
    );
}

#[tokio::test]
async fn cancelling_before_the_redirect_arrives_never_starts_a_token_exchange() {
    let server = MockTokenServer::start(r#"{"access_token":"should-never-be-fetched"}"#);
    let (opener, opened) = ChannelOpener::new();
    let cancel = CancelToken::new();
    let req = request(server.url.clone());

    let cancel_for_flow = cancel.clone();
    let flow = tokio::spawn(async move { authorize(req, &opener, &cancel_for_flow).await });

    // Wait for the flow to actually be listening before cancelling it, so
    // this test cannot pass by accident on a task that never started.
    opened.await.expect("the flow opens a browser URL");
    cancel.cancel();

    let err = flow
        .await
        .expect("the flow task joins")
        .expect_err("a cancelled flow produces no tokens");
    assert!(matches!(err, OAuthError::Cancelled));

    assert!(
        !server.was_contacted(),
        "a cancelled flow must never reach the token endpoint"
    );
}

#[tokio::test]
async fn a_mismatched_state_is_dropped_and_the_real_redirect_still_completes_the_flow() {
    let server = MockTokenServer::start(r#"{"access_token":"real-token"}"#);
    let (opener, opened) = ChannelOpener::new();
    let cancel = CancelToken::new();
    let req = request(server.url.clone());

    let flow = tokio::spawn(async move { authorize(req, &opener, &cancel).await });

    let authorize_url = opened.await.expect("the flow opens a browser URL");

    // A stray connection with the wrong state must not end the attempt or
    // trigger a token exchange...
    let pairs: std::collections::HashMap<_, _> = authorize_url.query_pairs().into_owned().collect();
    let redirect: Url = pairs["redirect_uri"].parse().unwrap();
    let mut bad = TcpStream::connect((redirect.host_str().unwrap(), redirect.port().unwrap()))
        .await
        .unwrap();
    bad.write_all(
        b"GET /?code=attacker&state=not-the-real-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await
    .unwrap();
    let mut discard = Vec::new();
    let _ = bad.read_to_end(&mut discard).await;

    // ...so the real browser tab still completes it.
    play_the_browser(&authorize_url, "the-real-code").await;

    let response = flow
        .await
        .expect("the flow task joins")
        .expect("the real redirect still succeeds");
    assert_eq!(response.access_token.expose(), "real-token");
}
