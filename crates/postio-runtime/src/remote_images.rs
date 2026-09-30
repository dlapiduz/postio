//! Remote images for a sender the user allowed (spec 006 FR-025, US5,
//! `contracts/remote-image-fetch.md`).
//!
//! POSTIO-CONSENT: a fetch starts only for a message the user opened, whose
//! sender is `Allowed` in the remote-image allow-list or for which the user
//! chose "Show once", and only for URLs that message's sanitized document
//! names as images. Nothing here prefetches: the reader's owner calls this
//! when a message opens, never when the list cursor passes one.
//!
//! The request is the plainest one that fetches a picture: `GET`, `Accept:
//! image/*`, a generic `User-Agent` that names neither Postio nor a version,
//! and no `Cookie`, `Referer`, `Origin` or `Authorization`. Redirects are
//! followed by hand, at most three, each target re-checked. A response is
//! capped in bytes and in time, and kept only if it sniffs as an image the
//! renderer decodes. Bytes live in memory for the process; nothing is
//! written to disk. Logs carry counts and outcomes, never a URL: a URL is
//! message content.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use io_http::client::{HttpClient, HttpClientError, HttpClientStd};
use io_http::rfc9110::request::HttpRequest;
use pimalaya_stream::stream::{Stream, TlsConnectOptions};
use url::Url;

/// The most a response may carry (research R4's image limit).
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
/// How long one image may take, connection to last byte.
pub const TIMEOUT: Duration = Duration::from_secs(10);
/// The most redirects one image may follow.
pub const MAX_REDIRECTS: usize = 3;
/// The most fetches in flight for one message.
pub const CONCURRENCY: usize = 4;
/// Says it is a browser-like client, and nothing about who.
const USER_AGENT: &str = "Mozilla/5.0";

/// Why an image did not arrive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// More than [`MAX_REDIRECTS`] redirects.
    TooManyRedirects,
    /// A redirect to something other than `http` or `https`.
    NotHttp,
    /// More than [`MAX_BYTES`].
    TooLarge,
    /// Longer than the timeout.
    TimedOut,
    /// Whatever it said it was, it is not a png, jpeg, gif or webp.
    NotAnImage,
    /// The server answered with this status.
    Status(u16),
    /// The connection could not be made or broke.
    Network,
}

/// What a fetch came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fetched {
    /// The image's bytes.
    Image(Arc<Vec<u8>>),
    /// Why not.
    Failed(Failure),
}

/// Fetches remote images and keeps them in memory for the process.
pub struct RemoteImageFetcher {
    cache: Mutex<HashMap<String, Fetched>>,
    timeout: Duration,
}

impl Default for RemoteImageFetcher {
    fn default() -> Self {
        RemoteImageFetcher::new()
    }
}

impl RemoteImageFetcher {
    /// A fetcher with the production timeout.
    pub fn new() -> RemoteImageFetcher {
        RemoteImageFetcher::with_timeout(TIMEOUT)
    }

    /// A fetcher whose images each have `timeout`.
    pub fn with_timeout(timeout: Duration) -> RemoteImageFetcher {
        RemoteImageFetcher {
            cache: Mutex::new(HashMap::new()),
            timeout,
        }
    }

    /// Fetch every one of one message's image URLs, at most
    /// [`CONCURRENCY`] at a time, from the cache where it can.
    pub async fn fetch_all(&self, urls: &[Url]) -> Vec<(Url, Fetched)> {
        let mut out = Vec::with_capacity(urls.len());
        for batch in urls.chunks(CONCURRENCY) {
            let mut tasks = Vec::new();
            for url in batch {
                if let Some(cached) = self.cached(url) {
                    out.push((url.clone(), cached));
                    continue;
                }
                let (url, timeout) = (url.clone(), self.timeout);
                tasks.push(tokio::task::spawn_blocking(move || {
                    let fetched = fetch_blocking(&url, timeout);
                    (url, fetched)
                }));
            }
            for task in tasks {
                if let Ok((url, fetched)) = task.await {
                    self.cache
                        .lock()
                        .expect("the image cache is never poisoned")
                        .insert(url.to_string(), fetched.clone());
                    out.push((url, fetched));
                }
            }
        }
        let arrived = out
            .iter()
            .filter(|(_, f)| matches!(f, Fetched::Image(_)))
            .count();
        tracing::debug!(
            urls = urls.len(),
            arrived,
            failed = out.len() - arrived,
            "remote images fetched"
        );
        out
    }

    fn cached(&self, url: &Url) -> Option<Fetched> {
        self.cache
            .lock()
            .expect("the image cache is never poisoned")
            .get(url.as_str())
            .cloned()
    }
}

fn fetch_blocking(url: &Url, timeout: Duration) -> Fetched {
    let deadline = Instant::now() + timeout;
    let mut url = url.clone();
    for _ in 0..=MAX_REDIRECTS {
        if !matches!(url.scheme(), "http" | "https") {
            return Fetched::Failed(Failure::NotHttp);
        }
        let Some(host) = url.host_str().map(str::to_owned) else {
            return Fetched::Failed(Failure::Network);
        };
        let stream = match connect(&url, &host, deadline) {
            Ok(stream) => stream,
            Err(failure) => return Fetched::Failed(failure),
        };
        let guard = Guard::new(stream, deadline);
        let over = guard.over.clone();
        let mut client = HttpClientStd::new(guard);
        let request = HttpRequest {
            method: "GET".to_owned(),
            url: url.clone(),
            headers: vec![
                ("Host".to_owned(), host),
                ("Accept".to_owned(), "image/*".to_owned()),
                ("User-Agent".to_owned(), USER_AGENT.to_owned()),
            ],
            body: Vec::new(),
        };
        match client.send(request) {
            Ok(out) => {
                let status = *out.response.status;
                if !out.response.status.is_success() {
                    return Fetched::Failed(Failure::Status(status));
                }
                let body = out.response.body;
                return if image_format(&body) {
                    Fetched::Image(Arc::new(body))
                } else {
                    Fetched::Failed(Failure::NotAnImage)
                };
            }
            Err(HttpClientError::UnexpectedRedirect { url: next, .. }) => url = next,
            Err(_) => {
                return Fetched::Failed(match *over.lock().expect("never poisoned") {
                    Some(failure) => failure,
                    None => Failure::Network,
                });
            }
        }
    }
    Fetched::Failed(Failure::TooManyRedirects)
}

/// Whether `bytes` are a png, jpeg, gif or webp, by their own signature.
fn image_format(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
}

trait ReadWriteSend: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWriteSend for T {}

fn connect(url: &Url, host: &str, deadline: Instant) -> Result<Box<dyn ReadWriteSend>, Failure> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(Failure::TimedOut);
    }
    match url.scheme() {
        "https" => {
            let options = TlsConnectOptions {
                tls: pimalaya_stream::tls::Tls {
                    rustls: pimalaya_stream::tls::Rustls {
                        alpn: vec!["http/1.1".into()],
                        ..Default::default()
                    },
                    ..Default::default()
                },
                retry: pimalaya_stream::retry::Retry::Until(left),
                ..Default::default()
            };
            let port = url.port_or_known_default().unwrap_or(443);
            Stream::connect_tls(host, port, options)
                .map(|s| Box::new(s) as Box<dyn ReadWriteSend>)
                .map_err(|_| Failure::Network)
        }
        _ => {
            let port = url.port_or_known_default().unwrap_or(80);
            let stream =
                std::net::TcpStream::connect((host, port)).map_err(|_| Failure::Network)?;
            stream
                .set_read_timeout(Some(left))
                .map_err(|_| Failure::Network)?;
            Ok(Box::new(stream))
        }
    }
}

/// A stream that refuses to read past the deadline or past the size cap,
/// and says which it was.
struct Guard {
    inner: Box<dyn ReadWriteSend>,
    deadline: Instant,
    read: usize,
    over: Arc<Mutex<Option<Failure>>>,
}

impl Guard {
    fn new(inner: Box<dyn ReadWriteSend>, deadline: Instant) -> Guard {
        Guard {
            inner,
            deadline,
            read: 0,
            over: Arc::new(Mutex::new(None)),
        }
    }

    fn refuse(&self, failure: Failure) -> std::io::Error {
        *self.over.lock().expect("never poisoned") = Some(failure);
        std::io::Error::other(format!("{failure:?}"))
    }
}

impl Read for Guard {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() > self.deadline {
            return Err(self.refuse(Failure::TimedOut));
        }
        let n = self.inner.read(buf).map_err(|err| {
            if matches!(
                err.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) {
                self.refuse(Failure::TimedOut)
            } else {
                err
            }
        })?;
        self.read += n;
        // The headers' allowance on top of the body's.
        if self.read > MAX_BYTES + 64 * 1024 {
            return Err(self.refuse(Failure::TooLarge));
        }
        Ok(n)
    }
}

impl Write for Guard {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000rest-of-a-png";

    /// A loopback server: each connection's request head is recorded, and
    /// `answer` decides the response for its path.
    struct Server {
        port: u16,
        heads: Arc<Mutex<Vec<String>>>,
        connections: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    impl Server {
        fn start(answer: fn(&str, u16) -> Vec<u8>, hold: Duration) -> Server {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
            let port = listener.local_addr().expect("an address").port();
            let heads = Arc::new(Mutex::new(Vec::new()));
            let connections = Arc::new(AtomicUsize::new(0));
            let live = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let (h, c, l, p) = (
                heads.clone(),
                connections.clone(),
                live.clone(),
                peak.clone(),
            );
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (h, l, p) = (h.clone(), l.clone(), p.clone());
                    c.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let now = l.fetch_add(1, Ordering::SeqCst) + 1;
                        p.fetch_max(now, Ordering::SeqCst);
                        let mut reader = BufReader::new(stream.try_clone().expect("a clone"));
                        let mut head = String::new();
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                                break;
                            }
                            head.push_str(&line);
                        }
                        let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
                        h.lock().expect("never poisoned").push(head);
                        std::thread::sleep(hold);
                        let mut stream = stream;
                        let _ = stream.write_all(&answer(&path, port));
                        l.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            });
            Server {
                port,
                heads,
                connections,
                peak,
            }
        }

        fn url(&self, path: &str) -> Url {
            Url::parse(&format!("http://127.0.0.1:{}{path}", self.port)).expect("a URL")
        }
    }

    fn ok(content_type: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn routes(path: &str, port: u16) -> Vec<u8> {
        let redirect = |to: String| {
            format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .into_bytes()
        };
        match path {
            "/image.png" => ok("image/png", PNG),
            "/fake.png" => ok("image/png", b"<html><body>not a picture</body></html>"),
            "/huge.png" => {
                let mut body = PNG.to_vec();
                // Well past the cap, headers' allowance and all.
                body.resize(MAX_BYTES + 512 * 1024, 0);
                ok("image/png", &body)
            }
            "/ftp" => redirect("ftp://127.0.0.1/image.png".to_owned()),
            p if p.starts_with("/hop/") => {
                let n: usize = p[5..].parse().unwrap_or(0);
                if n == 0 {
                    ok("image/png", PNG)
                } else {
                    redirect(format!("http://127.0.0.1:{port}/hop/{}", n - 1))
                }
            }
            _ => {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            }
        }
    }

    #[tokio::test]
    async fn an_image_is_fetched_once_and_kept_in_memory() {
        let server = Server::start(routes, Duration::ZERO);
        let fetcher = RemoteImageFetcher::new();
        let url = server.url("/image.png");
        let first = fetcher.fetch_all(std::slice::from_ref(&url)).await;
        assert_eq!(first[0].1, Fetched::Image(Arc::new(PNG.to_vec())));
        let _ = fetcher.fetch_all(std::slice::from_ref(&url)).await;
        assert_eq!(
            server.connections.load(Ordering::SeqCst),
            1,
            "the second fetch was not cached"
        );
    }

    #[tokio::test]
    async fn the_request_says_nothing_about_who_asked() {
        let server = Server::start(routes, Duration::ZERO);
        let fetcher = RemoteImageFetcher::new();
        let _ = fetcher.fetch_all(&[server.url("/image.png")]).await;
        let head = server.heads.lock().expect("never poisoned")[0].to_ascii_lowercase();
        for refused in ["cookie:", "referer:", "origin:", "authorization:"] {
            assert!(
                !head.contains(refused),
                "the request sent {refused}\n{head}"
            );
        }
        assert!(head.contains("user-agent:"), "{head}");
        assert!(
            !head.contains("postio"),
            "the user agent names Postio\n{head}"
        );
    }

    #[tokio::test]
    async fn each_limit_fails_with_its_reason() {
        let server = Server::start(routes, Duration::ZERO);
        let fetcher = RemoteImageFetcher::new();
        let cases = [
            ("/hop/3", Fetched::Image(Arc::new(PNG.to_vec()))),
            ("/hop/4", Fetched::Failed(Failure::TooManyRedirects)),
            ("/ftp", Fetched::Failed(Failure::NotHttp)),
            ("/huge.png", Fetched::Failed(Failure::TooLarge)),
            ("/fake.png", Fetched::Failed(Failure::NotAnImage)),
            ("/missing.png", Fetched::Failed(Failure::Status(404))),
        ];
        for (path, expected) in cases {
            let got = fetcher.fetch_all(&[server.url(path)]).await;
            // Compared by kind: a failure's bytes can be 16 MiB of noise.
            let kind = |f: &Fetched| match f {
                Fetched::Image(bytes) => format!("image of {} bytes", bytes.len()),
                Fetched::Failed(why) => format!("{why:?}"),
            };
            assert_eq!(kind(&got[0].1), kind(&expected), "{path}");
        }
        let slow = Server::start(routes, Duration::from_millis(600));
        let impatient = RemoteImageFetcher::with_timeout(Duration::from_millis(150));
        let got = impatient.fetch_all(&[slow.url("/image.png")]).await;
        assert_eq!(got[0].1, Fetched::Failed(Failure::TimedOut));
    }

    #[tokio::test]
    async fn at_most_four_fetches_are_in_flight_for_a_message() {
        let server = Server::start(routes, Duration::from_millis(100));
        let fetcher = RemoteImageFetcher::new();
        let urls: Vec<Url> = (0..10)
            .map(|n| server.url(&format!("/image.png?n={n}")))
            .collect();
        let got = fetcher.fetch_all(&urls).await;
        assert_eq!(got.len(), 10);
        assert!(
            server.peak.load(Ordering::SeqCst) <= CONCURRENCY,
            "{} at once",
            server.peak.load(Ordering::SeqCst)
        );
    }
}
