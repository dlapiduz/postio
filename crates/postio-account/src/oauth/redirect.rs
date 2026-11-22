//! ADR 0006 Q3: bound for one attempt, not a custom URI scheme, which needs desktop-file
//! registration and hands the callback to whatever else claimed it.
//! A mismatched-`state` or unrecognizable connection is answered and dropped without ending the
//! attempt, so a stray local probe cannot abort a real sign-in.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use super::error::OAuthError;
use super::pkce::State;
use crate::cancel::CancelToken;

pub const REDIRECT_IO_TIMEOUT: Duration = Duration::from_secs(5);

const MAX_REQUEST_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationCode(String);

impl AuthorizationCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct LoopbackRedirect {
    listener: TcpListener,
    port: u16,
}

impl LoopbackRedirect {
    /// Loopback only, never `0.0.0.0`.
    pub async fn bind() -> Result<Self, OAuthError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(OAuthError::Bind)?;
        let port = listener.local_addr().map_err(OAuthError::Bind)?.port();
        Ok(Self { listener, port })
    }

    pub fn redirect_uri(&self) -> Url {
        format!("http://127.0.0.1:{}/", self.port)
            .parse()
            .expect("a loopback URL with a bound port always parses")
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Consumes `self`: the listener's socket closes on any return, so it cannot be waited on
    /// twice.
    pub async fn wait_for_code(
        self,
        expected: &State,
        cancel: &CancelToken,
    ) -> Result<AuthorizationCode, OAuthError> {
        loop {
            let stream = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(OAuthError::Cancelled),
                accepted = self.listener.accept() => accepted.map_err(OAuthError::Redirect)?.0,
            };

            match tokio::select! {
                _ = cancel.cancelled() => return Err(OAuthError::Cancelled),
                outcome = handle_connection(stream, expected) => outcome,
            } {
                Outcome::Code(code) => return Ok(code),
                Outcome::Denied(reason) => return Err(OAuthError::Denied(reason)),
                Outcome::Ignored => continue,
            }
        }
    }
}

enum Outcome {
    Code(AuthorizationCode),
    Denied(String),
    Ignored,
}

async fn handle_connection(mut stream: TcpStream, expected: &State) -> Outcome {
    let request =
        match tokio::time::timeout(REDIRECT_IO_TIMEOUT, read_request_line(&mut stream)).await {
            Ok(Ok(line)) => line,
            _ => return Outcome::Ignored,
        };

    let Some(path_and_query) = request.split_whitespace().nth(1) else {
        respond(&mut stream, 400, "Malformed request.").await;
        return Outcome::Ignored;
    };

    let Ok(url) = Url::parse(&format!("http://127.0.0.1{path_and_query}")) else {
        respond(&mut stream, 400, "Malformed request.").await;
        return Outcome::Ignored;
    };

    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();

    if let Some(error) = params.get("error") {
        let reason = params
            .get("error_description")
            .map(|d| format!("{error}: {d}"))
            .unwrap_or_else(|| error.clone());
        respond(
            &mut stream,
            200,
            "Sign-in was declined. You can close this tab.",
        )
        .await;
        return Outcome::Denied(reason);
    }

    let (Some(code), Some(state)) = (params.get("code"), params.get("state")) else {
        respond(&mut stream, 400, "Missing code or state.").await;
        return Outcome::Ignored;
    };

    if expected.as_str() != state.as_str() {
        // Answered without confirming the mismatch: a stray local probe or a stale tab should not
        // learn it interfered.
        respond(
            &mut stream,
            400,
            "This sign-in attempt is no longer active.",
        )
        .await;
        return Outcome::Ignored;
    }

    respond(
        &mut stream,
        200,
        "Sign-in complete. You can close this tab and return to Postio.",
    )
    .await;
    Outcome::Code(AuthorizationCode(code.clone()))
}

async fn read_request_line(stream: &mut TcpStream) -> io::Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if buf.len() >= MAX_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request too large",
            ));
        }
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed early",
            ));
        }
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buf)
        .trim_end_matches('\r')
        .to_string())
}

/// Best-effort: the browser tab is a courtesy, not part of the protocol.
async fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    let body = format!("<!doctype html><html><body><p>{message}</p></body></html>");
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = tokio::time::timeout(REDIRECT_IO_TIMEOUT, stream.write_all(response.as_bytes())).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn send_redirect(port: u16, query: &str) {
        let mut stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect to the loopback listener");
        stream
            .write_all(format!("GET /?{query} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .expect("write the redirect request");
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;
        assert!(!response.is_empty(), "the listener should answer");
    }

    #[tokio::test]
    async fn a_matching_redirect_yields_its_code() {
        let listener = LoopbackRedirect::bind().await.expect("bind");
        let port = listener.port();
        let state = State::generate();
        let cancel = CancelToken::new();

        let expected_state = state.clone();
        let waiting =
            tokio::spawn(async move { listener.wait_for_code(&expected_state, &cancel).await });

        send_redirect(port, &format!("code=the-code&state={}", state.as_str())).await;

        let code = waiting.await.expect("task joins").expect("a code arrives");
        assert_eq!(code.as_str(), "the-code");
    }

    #[tokio::test]
    async fn a_state_mismatch_is_dropped_without_ending_the_attempt() {
        let listener = LoopbackRedirect::bind().await.expect("bind");
        let port = listener.port();
        let state = State::generate();
        let cancel = CancelToken::new();

        let expected_state = state.clone();
        let waiting =
            tokio::spawn(async move { listener.wait_for_code(&expected_state, &cancel).await });

        // A stray/attacker connection with the wrong state...
        send_redirect(port, "code=attacker-code&state=not-the-real-one").await;
        // ...must not have ended the attempt: the real browser tab still
        // completes it.
        send_redirect(
            port,
            &format!("code=the-real-code&state={}", state.as_str()),
        )
        .await;

        let code = waiting
            .await
            .expect("task joins")
            .expect("the real code arrives");
        assert_eq!(code.as_str(), "the-real-code");
    }

    #[tokio::test]
    async fn a_provider_denial_ends_the_attempt_with_the_reason() {
        let listener = LoopbackRedirect::bind().await.expect("bind");
        let port = listener.port();
        let state = State::generate();
        let cancel = CancelToken::new();

        let expected_state = state.clone();
        let waiting =
            tokio::spawn(async move { listener.wait_for_code(&expected_state, &cancel).await });

        send_redirect(
            port,
            &format!("error=access_denied&state={}", state.as_str()),
        )
        .await;

        let err = waiting
            .await
            .expect("task joins")
            .expect_err("denial is an error");
        assert!(matches!(err, OAuthError::Denied(reason) if reason.contains("access_denied")));
    }

    #[tokio::test]
    async fn cancelling_stops_the_wait_and_closes_the_listener() {
        let listener = LoopbackRedirect::bind().await.expect("bind");
        let port = listener.port();
        let state = State::generate();
        let cancel = CancelToken::new();

        let waiting_cancel = cancel.clone();
        let waiting =
            tokio::spawn(async move { listener.wait_for_code(&state, &waiting_cancel).await });

        tokio::task::yield_now().await;
        cancel.cancel();

        let err = waiting
            .await
            .expect("task joins")
            .expect_err("cancellation is an error");
        assert!(matches!(err, OAuthError::Cancelled));

        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }
}
