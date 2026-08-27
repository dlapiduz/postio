//! Reaching the authorization server: RFC 8414 metadata, and the token POST.
//!
//! ADR 0006 Q3: "the token exchange rides the same HTTP transport discovery
//! already uses" — [`crate::discovery::transport`]'s TCP/TLS connect
//! helpers, cancellable the same way an autoconfig fetch is. `io-http`'s
//! [`HttpClientStd`] drives the actual request/response cycle once a stream
//! is open; it is already in the dependency graph underneath
//! `io-pim-discovery`; nothing new arrives for it.
//!
//! The grant bodies and token responses are `io-oauth`'s RFC 6749 types
//! (#537): the form encodings, the success/error schemas and their secrecy
//! handling are maintained upstream, in the same I/O-free style as every
//! other Pimalaya wire crate here. What stays this module's: the transport
//! pump above (io-oauth's optional client wrapper consumes the HTTP status,
//! and the shim below needs it), and the mapping into [`OAuthError`].

use std::io::{Read, Write};
use std::time::Duration;

use io_http::client::{HttpClient, HttpClientStd};
use io_http::rfc9110::request::HttpRequest;
use io_oauth::rfc6749::access_token_request::Oauth20AccessTokenRequestParams;
use io_oauth::rfc6749::issue_access_token::Oauth20AccessTokenSuccessParams;
use io_oauth::rfc6749::refresh_access_token::Oauth20AccessTokenRefreshParams;
use io_pim_discovery::rfc8414::DiscoveryOauthServerMetadata;
use pimalaya_stream::stream::{Stream, TlsConnectOptions};
use secrecy::ExposeSecret;
use url::Url;

use super::error::OAuthError;
use crate::cancel::CancelToken;
use crate::discovery::transport::{self, PimalayaTransport};
use crate::secret::Password;

/// How long a request to an authorization server may run. Generous relative
/// to [`crate::discovery::transport::DISCOVERY_IO_TIMEOUT`] because a token
/// endpoint does real work (validating a code, minting a signed token)
/// rather than serving a static document, but still a bound: this is a
/// user-initiated foreground wait, not a background job that can simply
/// keep retrying.
pub const REQUEST_IO_TIMEOUT: Duration = Duration::from_secs(15);

// ---------------------------------------------------------------------------
// Endpoint resolution (RFC 8414)
// ---------------------------------------------------------------------------

/// The two endpoints an authorization flow needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    /// Where consent is asked.
    pub authorize: Url,
    /// Where a code (or a refresh token) is exchanged.
    pub token: Url,
}

/// Resolves `issuer`'s authorization and token endpoints via RFC 8414
/// metadata, falling back to the OpenID Connect Discovery compatibility
/// document (RFC 8414 §5) when the server only publishes that one.
pub async fn resolve_endpoints(
    issuer: &Url,
    cancel: &CancelToken,
) -> Result<Endpoints, OAuthError> {
    let primary = DiscoveryOauthServerMetadata::well_known_url(issuer);
    match fetch_metadata(&primary, cancel).await {
        Ok(metadata) => endpoints_from(metadata),
        Err(_) => {
            let fallback = DiscoveryOauthServerMetadata::openid_well_known_url(issuer);
            let metadata = fetch_metadata(&fallback, cancel).await?;
            endpoints_from(metadata)
        }
    }
}

fn endpoints_from(metadata: DiscoveryOauthServerMetadata) -> Result<Endpoints, OAuthError> {
    Ok(Endpoints {
        authorize: metadata
            .authorization_endpoint
            .ok_or(OAuthError::MissingEndpoint("authorization"))?,
        token: metadata
            .token_endpoint
            .ok_or(OAuthError::MissingEndpoint("token"))?,
    })
}

async fn fetch_metadata(
    url: &Url,
    cancel: &CancelToken,
) -> Result<DiscoveryOauthServerMetadata, OAuthError> {
    let host = url
        .host_str()
        .ok_or_else(|| OAuthError::Http(format!("URL `{url}` has no host")))?
        .to_string();
    let request = HttpRequest::get(url.clone())
        .header("Host", host)
        .header("Accept", "application/json");

    let body = send(url.clone(), request, cancel.clone()).await?;
    DiscoveryOauthServerMetadata::try_from(body.as_slice())
        .map_err(|err| OAuthError::Parse(err.to_string()))
}

// ---------------------------------------------------------------------------
// Token exchange
// ---------------------------------------------------------------------------

/// What one successful token response carries.
///
/// `access_token` and `refresh_token` are [`Password`]s — zeroizing,
/// redacted in `Debug` — the same protection every other credential in this
/// crate gets. ADR 0006 Q3: the refresh token goes to the keyring, the
/// access token stays in memory and is never written to disk; this type is
/// what a caller stores and caches, not this module's business.
#[derive(Debug)]
pub struct TokenResponse {
    pub access_token: Password,
    pub refresh_token: Option<Password>,
    /// How long `access_token` is valid for, from the moment this response
    /// was received.
    pub expires_in: Option<Duration>,
    pub token_type: String,
    pub scope: Option<String>,
}

/// Parameters for an authorization-code grant (RFC 6749 §4.1.3).
pub struct CodeExchange<'a> {
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub code: &'a str,
    pub code_verifier: &'a str,
    pub redirect_uri: &'a str,
}

/// Exchanges an authorization code for tokens.
pub async fn exchange_code(
    token_url: &Url,
    params: CodeExchange<'_>,
    cancel: &CancelToken,
) -> Result<TokenResponse, OAuthError> {
    // io-oauth's own RFC 6749 §4.1.3 body. Built to a `String` before the
    // `.await` below: the params' serializer is not `Send`, and every
    // `TokenSource` impl is required to be `Send + Sync`.
    let form_body = {
        use std::str::FromStr;
        let verifier =
            io_oauth::rfc7636::pkce::Oauth20PkceCodeVerifier::from_str(params.code_verifier)
                .map_err(|byte| {
                    OAuthError::Parse(format!(
                        "the PKCE verifier carries a byte RFC 7636 does not allow: 0x{byte:x}"
                    ))
                })?;
        Oauth20AccessTokenRequestParams {
            code: params.code.into(),
            redirect_uri: Some(params.redirect_uri.into()),
            client_id: params.client_id.into(),
            client_secret: params.client_secret.map(|secret| secret.to_owned().into()),
            pkce_code_verifier: Some(std::borrow::Cow::Owned(verifier)),
        }
        .to_string()
    };

    token_request(token_url, form_body, cancel).await
}

/// Parameters for a refresh-token grant (RFC 6749 §6).
pub struct RefreshExchange<'a> {
    pub client_id: &'a str,
    pub client_secret: Option<&'a str>,
    pub refresh_token: &'a str,
}

/// Exchanges a refresh token for a fresh access token.
pub async fn refresh_token(
    token_url: &Url,
    params: RefreshExchange<'_>,
    cancel: &CancelToken,
) -> Result<TokenResponse, OAuthError> {
    // io-oauth's RFC 6749 §6 body, same `String`-before-await rule as the
    // code exchange.
    let form_body = {
        let mut body =
            Oauth20AccessTokenRefreshParams::new(params.client_id, params.refresh_token.to_owned());
        body.client_secret = params.client_secret.map(|secret| secret.to_owned().into());
        body.to_string()
    };

    token_request(token_url, form_body, cancel).await
}

async fn token_request(
    token_url: &Url,
    form_body: String,
    cancel: &CancelToken,
) -> Result<TokenResponse, OAuthError> {
    let host = token_url
        .host_str()
        .ok_or_else(|| OAuthError::Http(format!("URL `{token_url}` has no host")))?
        .to_string();
    let request = HttpRequest {
        method: "POST".to_string(),
        url: token_url.clone(),
        headers: vec![
            ("Host".to_string(), host),
            (
                "Content-Type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            ),
            ("Accept".to_string(), "application/json".to_string()),
        ],
        body: form_body.into_bytes(),
    };

    let body = send(token_url.clone(), request, cancel.clone()).await?;
    parse_token_response(&body)
}

/// The error shape, read with the provider's own strings intact.
///
/// io-oauth's `Oauth20AccessTokenErrorParams` reads the same shape, but its
/// error code is a closed enum with an `Unknown` catch-all — a
/// provider-specific code would reach the user as "unknown" instead of the
/// string the provider actually sent, and that string is often the only
/// clue in a support thread. Five lines of fidelity are worth keeping.
#[derive(serde::Deserialize)]
struct RawTokenError {
    error: String,
    error_description: Option<String>,
}

/// Reads a token endpoint's body: the error shape first, then io-oauth's
/// RFC 6749 §5.1 success schema.
///
/// The error shape is checked **regardless of the HTTP status**: §5.2
/// allows statuses other than 400, and at least one provider in the wild
/// answers `200 OK` with `{"error": …}`. io-oauth's optional client trusts
/// the status to pick the schema, which is exactly why the pump above
/// stays ours.
fn parse_token_response(body: &[u8]) -> Result<TokenResponse, OAuthError> {
    if let Ok(err) = serde_json::from_slice::<RawTokenError>(body) {
        let reason = err
            .error_description
            .map(|d| format!("{}: {d}", err.error))
            .unwrap_or(err.error);
        return Err(OAuthError::Status {
            status: 200,
            body: reason,
        });
    }

    // Liberal in what is accepted, deliberately: RFC 6749 §5.1 marks
    // `token_type` REQUIRED and io-oauth enforces that; refresh responses
    // in the wild omit it, and the old parser defaulted it to `Bearer`
    // for exactly that reason. The default is restored here rather than
    // lost to the swap — a candidate upstream change, noted on #537.
    let normalized: Vec<u8>;
    let body = match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(serde_json::Value::Object(mut map)) if !map.contains_key("token_type") => {
            map.insert("token_type".to_owned(), "Bearer".into());
            normalized =
                serde_json::to_vec(&map).map_err(|err| OAuthError::Parse(err.to_string()))?;
            normalized.as_slice()
        }
        _ => body,
    };
    let raw = Oauth20AccessTokenSuccessParams::try_from(body)
        .map_err(|err| OAuthError::Parse(err.to_string()))?;

    Ok(TokenResponse {
        access_token: Password::new(raw.access_token.expose_secret().to_owned()),
        refresh_token: raw
            .refresh_token
            .map(|token| Password::new(token.expose_secret().to_owned())),
        expires_in: raw
            .expires_in
            .map(|seconds| Duration::from_secs(seconds as u64)),
        token_type: raw.token_type,
        scope: raw.scope,
    })
}

// ---------------------------------------------------------------------------
// The HTTP request/response cycle
// ---------------------------------------------------------------------------

/// Opens a stream to `url`'s origin and sends `request` on it, returning the
/// response body on a success status.
async fn send(url: Url, request: HttpRequest, cancel: CancelToken) -> Result<Vec<u8>, OAuthError> {
    tokio::task::spawn_blocking(move || send_blocking(&url, request, cancel))
        .await
        .map_err(|err| OAuthError::Http(format!("the request task did not finish: {err}")))?
}

fn send_blocking(
    url: &Url,
    request: HttpRequest,
    cancel: CancelToken,
) -> Result<Vec<u8>, OAuthError> {
    let stream = connect(url, &cancel)?;
    let mut client = HttpClientStd::new(stream);

    let out = client
        .send(request)
        .map_err(|err| OAuthError::Http(err.to_string()))?;

    let status = *out.response.status;
    if !out.response.status.is_success() {
        return Err(OAuthError::Status {
            status,
            body: String::from_utf8_lossy(&out.response.body).into_owned(),
        });
    }
    Ok(out.response.body)
}

/// Opens a cancellable, bounded stream to `url`'s origin — TLS for
/// `https`, plain for `http` (only ever reached in tests, against a mock
/// server on loopback).
fn connect(url: &Url, cancel: &CancelToken) -> Result<Box<dyn ReadWriteSend>, OAuthError> {
    let host = url
        .host_str()
        .ok_or_else(|| OAuthError::Http(format!("URL `{url}` has no host")))?;

    match url.scheme() {
        "https" => {
            let port = url.port_or_known_default().unwrap_or(443);
            let options = TlsConnectOptions {
                tls: PimalayaTransport::tls(),
                retry: pimalaya_stream::retry::Retry::Until(REQUEST_IO_TIMEOUT),
                ..Default::default()
            };
            let stream = Stream::connect_tls(host, port, options)
                .map_err(|err| OAuthError::Http(err.to_string()))?;
            Ok(Box::new(transport::Cancellable::new(
                stream,
                cancel.clone(),
            )))
        }
        "http" => {
            let port = url.port_or_known_default().unwrap_or(80);
            let stream = Stream::connect_tcp(host, port, transport::connect_options())
                .map_err(|err| OAuthError::Http(err.to_string()))?;
            Ok(Box::new(transport::Cancellable::new(
                stream,
                cancel.clone(),
            )))
        }
        other => Err(OAuthError::Http(format!(
            "unsupported scheme `{other}` (expected `http` or `https`)"
        ))),
    }
}

/// Marker alias so [`connect`]'s return type reads as one thing rather than
/// three bounds repeated at every call site.
trait ReadWriteSend: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWriteSend for T {}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    /// A one-shot mock authorization server: accepts one connection, reads
    /// one HTTP/1.1 request, and answers with `body` and `status`. Runs on
    /// a background thread so the async test can drive real, cancellable
    /// I/O against it over plain loopback TCP — this crate's `oauth`
    /// requests never touch a real network in the default suite.
    struct MockServer {
        url: Url,
        handle: Option<thread::JoinHandle<Vec<u8>>>,
    }

    impl MockServer {
        fn start(status: &'static str, body: &'static str) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let port = listener.local_addr().expect("local addr").port();
            let url: Url = format!("http://127.0.0.1:{port}/token").parse().unwrap();

            let handle = thread::spawn(move || {
                let (stream, _) = listener.accept().expect("accept one connection");
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request_line = String::new();
                reader
                    .read_line(&mut request_line)
                    .expect("read request line");

                let mut content_length = 0usize;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).expect("read header line");
                    let header = header.trim_end();
                    if header.is_empty() {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body_bytes = vec![0u8; content_length];
                std::io::Read::read_exact(&mut reader, &mut body_bytes).expect("read body");

                let mut stream = stream;
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");

                request_line
                    .into_bytes()
                    .into_iter()
                    .chain(body_bytes)
                    .collect()
            });

            Self {
                url,
                handle: Some(handle),
            }
        }

        /// Joins the server thread and returns the raw bytes it captured
        /// (the request line, then the body it read).
        fn join(mut self) -> Vec<u8> {
            self.handle
                .take()
                .expect("join once")
                .join()
                .expect("server thread panics on failure")
        }
    }

    #[tokio::test]
    async fn a_code_exchange_against_a_mock_server_returns_its_tokens() {
        let server = MockServer::start(
            "200 OK",
            r#"{"access_token":"abc123","refresh_token":"r-xyz","expires_in":3600,"token_type":"Bearer"}"#,
        );
        let url = server.url.clone();
        let cancel = CancelToken::new();

        let response = exchange_code(
            &url,
            CodeExchange {
                client_id: "client-1",
                client_secret: None,
                code: "the-code",
                code_verifier: "the-verifier",
                redirect_uri: "http://127.0.0.1:1/",
            },
            &cancel,
        )
        .await
        .expect("exchange succeeds");

        assert_eq!(response.access_token.expose(), "abc123");
        assert_eq!(
            response.refresh_token.as_ref().map(Password::expose),
            Some("r-xyz")
        );
        assert_eq!(response.expires_in, Some(Duration::from_secs(3600)));

        let captured = String::from_utf8(server.join()).expect("utf8");
        assert!(captured.starts_with("POST /token HTTP/1.1"), "{captured}");
        assert!(
            captured.contains("grant_type=authorization_code"),
            "{captured}"
        );
        assert!(captured.contains("code=the-code"), "{captured}");
        assert!(
            captured.contains("code_verifier=the-verifier"),
            "{captured}"
        );
    }

    #[tokio::test]
    async fn a_refresh_exchange_sends_the_refresh_token_grant() {
        let server = MockServer::start(
            "200 OK",
            r#"{"access_token":"fresh-token","expires_in":60}"#,
        );
        let url = server.url.clone();
        let cancel = CancelToken::new();

        let response = refresh_token(
            &url,
            RefreshExchange {
                client_id: "client-1",
                client_secret: None,
                refresh_token: "the-refresh-token",
            },
            &cancel,
        )
        .await
        .expect("refresh succeeds");

        assert_eq!(response.access_token.expose(), "fresh-token");
        assert!(response.refresh_token.is_none());

        let captured = String::from_utf8(server.join()).expect("utf8");
        assert!(captured.contains("grant_type=refresh_token"), "{captured}");
        assert!(
            captured.contains("refresh_token=the-refresh-token"),
            "{captured}"
        );
    }

    #[tokio::test]
    async fn a_non_success_status_is_reported_with_its_body() {
        let server = MockServer::start(
            "400 Bad Request",
            r#"{"error":"invalid_grant","error_description":"code expired"}"#,
        );
        let url = server.url.clone();
        let cancel = CancelToken::new();

        let err = exchange_code(
            &url,
            CodeExchange {
                client_id: "client-1",
                client_secret: None,
                code: "stale-code",
                code_verifier: "v",
                redirect_uri: "http://127.0.0.1:1/",
            },
            &cancel,
        )
        .await
        .expect_err("a 400 is an error");

        let OAuthError::Status { body, .. } = err else {
            panic!("expected Status, got {err:?}");
        };
        assert!(body.contains("invalid_grant"), "{body}");
        assert!(body.contains("code expired"), "{body}");

        server.join();
    }

    #[tokio::test]
    async fn a_200_with_an_error_body_is_still_reported_as_an_error() {
        // RFC 6749 §5.2 allows other statuses, but a token endpoint that
        // answers 200 with an `error` field has been seen in the wild —
        // the error shape must win regardless of the HTTP status.
        let server = MockServer::start("200 OK", r#"{"error":"invalid_client"}"#);
        let url = server.url.clone();
        let cancel = CancelToken::new();

        let err = exchange_code(
            &url,
            CodeExchange {
                client_id: "client-1",
                client_secret: None,
                code: "c",
                code_verifier: "v",
                redirect_uri: "http://127.0.0.1:1/",
            },
            &cancel,
        )
        .await
        .expect_err("a 200 carrying `error` is still an error");

        assert!(matches!(err, OAuthError::Status { .. }));
        server.join();
    }

    #[tokio::test]
    async fn a_cancelled_exchange_never_completes() {
        // Bind a server that never accepts, so the connect attempt is the
        // thing cancellation has to interrupt.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener
            .set_nonblocking(true)
            .expect("nonblocking so the test thread does not block on accept");
        let port = listener.local_addr().expect("addr").port();
        let url: Url = format!("http://127.0.0.1:{port}/token").parse().unwrap();

        let cancel = CancelToken::new();
        cancel.cancel();

        // Cancelled before the request starts: `Cancellable::check` refuses
        // the first read or write, so this must fail rather than hang or
        // succeed against a server that was never driven.
        let err = exchange_code(
            &url,
            CodeExchange {
                client_id: "client-1",
                client_secret: None,
                code: "c",
                code_verifier: "v",
                redirect_uri: "http://127.0.0.1:1/",
            },
            &cancel,
        )
        .await
        .expect_err("a pre-cancelled exchange must not succeed");

        assert!(matches!(err, OAuthError::Http(_)));
    }

    // -- RFC 8414 endpoint resolution ---------------------------------------

    #[tokio::test]
    async fn resolve_endpoints_reads_authorization_and_token_urls_from_metadata() {
        let metadata = r#"{
            "issuer": "https://example.com",
            "authorization_endpoint": "https://example.com/authorize",
            "token_endpoint": "https://example.com/token"
        }"#;
        let server = MockServer::start("200 OK", metadata);
        let issuer: Url = format!("http://127.0.0.1:{}", server.url.port().unwrap())
            .parse()
            .unwrap();
        let cancel = CancelToken::new();

        let endpoints = resolve_endpoints(&issuer, &cancel).await.expect("resolves");

        assert_eq!(
            endpoints.authorize.as_str(),
            "https://example.com/authorize"
        );
        assert_eq!(endpoints.token.as_str(), "https://example.com/token");
        server.join();
    }

    #[tokio::test]
    async fn resolve_endpoints_reports_which_endpoint_the_metadata_left_out() {
        let metadata = r#"{"issuer":"https://example.com","authorization_endpoint":"https://example.com/authorize"}"#;
        let server = MockServer::start("200 OK", metadata);
        let issuer: Url = format!("http://127.0.0.1:{}", server.url.port().unwrap())
            .parse()
            .unwrap();
        let cancel = CancelToken::new();

        let err = resolve_endpoints(&issuer, &cancel)
            .await
            .expect_err("a metadata document missing `token_endpoint` cannot resolve");

        assert!(matches!(err, OAuthError::MissingEndpoint("token")));
        server.join();
    }

    #[tokio::test]
    async fn resolve_endpoints_falls_back_to_the_openid_discovery_document() {
        // A server publishing only the OpenID Connect Discovery document:
        // the RFC 8414 well-known path 404s, the OIDC compatibility path
        // (RFC 8414 §5) answers.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let handle = thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut request_line = String::new();
                reader.read_line(&mut request_line).expect("request line");
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).expect("header line");
                    if header.trim_end().is_empty() {
                        break;
                    }
                }

                let (status, body) = if request_line.contains("oauth-authorization-server") {
                    ("404 Not Found", "not found".to_string())
                } else {
                    (
                        "200 OK",
                        r#"{"issuer":"https://example.com","authorization_endpoint":"https://example.com/authorize","token_endpoint":"https://example.com/token"}"#.to_string(),
                    )
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        let issuer: Url = format!("http://127.0.0.1:{port}").parse().unwrap();
        let cancel = CancelToken::new();

        let endpoints = resolve_endpoints(&issuer, &cancel)
            .await
            .expect("falls back to the OIDC document and resolves");

        assert_eq!(endpoints.token.as_str(), "https://example.com/token");
        handle.join().expect("server thread panics on failure");
    }
}
