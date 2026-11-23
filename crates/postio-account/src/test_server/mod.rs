//! A stateful IMAP server on loopback for the real client stack, since the mock sits above
//! `io-imap` and a scripted transcript cannot model changing state.
//! Fault injection is the point: `Quirk` is what a server is, `Fault` what it does once. Unknown
//! commands answer `BAD` so a client mistake does not pass; there is no TLS.

mod mime;
#[cfg(feature = "imap")]
mod pipe;
mod session;
mod state;
mod wire;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use postio_model::{FlagSet, ModSeq, Uid, UidValidity};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use self::state::{Mailbox, ServerState};
use self::wire::{Conn, WireStats};

#[cfg(feature = "imap")]
pub use self::pipe::InMemoryConnector;

const DEFAULT_ACCOUNT: &str = "someone@example.com";

const DEFAULT_PASSWORD: &str = "app-specific-password";

/// Deliberately unlike the password: presenting one where the other belongs must fail.
const DEFAULT_ACCESS_TOKEN: &str = "test-access-token";

/// Hides `CONDSTORE`, `QRESYNC`, `IDLE` and `UIDPLUS` until login, like the provider Postio targets
/// (ADR 0001 Q3).
const DEFAULT_BANNER: [&str; 4] = ["IMAP4rev1", "SASL-IR", "AUTH=PLAIN", "AUTH=LOGIN"];

const DEFAULT_CAPABILITIES: [&str; 14] = [
    "IMAP4rev1",
    "SASL-IR",
    "AUTH=PLAIN",
    "AUTH=LOGIN",
    "ENABLE",
    "CONDSTORE",
    "QRESYNC",
    "IDLE",
    "UIDPLUS",
    "MOVE",
    "NAMESPACE",
    "UNSELECT",
    "ID",
    "COMPRESS=DEFLATE",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Quirk {
    /// The conformant behaviour, not the default: hiding extensions until after authentication is
    /// what the provider Postio targets does.
    AdvertiseExtensionsBeforeLogin,

    /// Without it, the default, the client has to ask again: the path `ensure_capabilities` exists
    /// for.
    CapabilityInLoginResponse,

    /// A violation of RFC 5161 §3.1 shipped by at least one mainstream provider: a client gating
    /// QRESYNC on the echo silently loses incremental sync.
    OmitEnabledEcho,

    /// A sequence number is a `NonZeroU32`, so the line cannot be decoded; `io-imap` skips it and
    /// completes `Ok`, so a resync that lost a message's flags looks complete.
    MalformedFetchSequenceNumber,

    /// The server did send it and the client lost it: `io-imap` skips the undecodable line and
    /// completes `Ok`, indistinguishable from a server that never sends one unless the skip is
    /// counted.
    UidValidityLostToAnUndecodableLine,

    /// A live account contradicted itself across `SELECT`s (#1538); a client believing the first
    /// answer throws away the folder's whole UID space.
    UidValidityFlapsOnce { generation: u32 },

    /// The other half of #1538: RFC 3501 §6.3.1 requires one, but giving up on the first omission
    /// loses the folder for that pass.
    UidValidityOmittedOnce,

    /// A broken server that must still be refused: a mailbox whose generation is unknowable cannot
    /// be addressed by UID.
    UidValidityAlwaysOmitted,

    /// The `+` goes out and nothing is read again, so a client writing a large `APPEND` blocks in
    /// its write, which no read deadline sees.
    DeafToLiterals,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// Against `FETCH` it tears the connection mid body literal, after the client was told how many
    /// octets are coming.
    DropConnection {
        during: String,
    },
    Stall {
        during: String,
    },
    /// A client bounding how long a command may take kills this connection; one bounding how long
    /// the server may be silent keeps it, as a large attachment over a slow link needs.
    Trickle {
        during: String,
        gap: Duration,
    },
    /// Not `Stall`: the client has read part of the literal and waits with a half-built response in
    /// its parser.
    StallMidLiteral {
        during: String,
    },
    IgnoreDone,
    Reject {
        during: String,
        reason: String,
    },
}

impl Fault {
    fn matches(&self, command: &str) -> bool {
        let during = match self {
            Self::IgnoreDone => return command.eq_ignore_ascii_case("IDLE"),
            Self::DropConnection { during }
            | Self::Stall { during }
            | Self::StallMidLiteral { during }
            | Self::Trickle { during, .. }
            | Self::Reject { during, .. } => during,
        };
        during.eq_ignore_ascii_case(command)
    }
}

#[derive(Clone, Debug)]
pub struct TestMessage {
    raw: Vec<u8>,
    flags: FlagSet,
    internal_date: Option<DateTime<Utc>>,
}

impl TestMessage {
    pub fn new(raw: impl Into<Vec<u8>>) -> Self {
        Self {
            raw: raw.into(),
            flags: FlagSet::new(),
            internal_date: None,
        }
    }

    pub fn corpus(name: &str) -> Self {
        Self::new(postio_model::test_corpus::load(name).bytes().to_vec())
    }

    pub fn with_flags(mut self, flags: FlagSet) -> Self {
        self.flags = flags;
        self
    }

    pub fn with_internal_date(mut self, at: DateTime<Utc>) -> Self {
        self.internal_date = Some(at);
        self
    }
}

#[derive(Clone, Debug)]
pub struct TestMailbox {
    path: String,
    delimiter: char,
    attributes: Vec<String>,
    subscribed: bool,
    uid_validity: UidValidity,
    highest_mod_seq: ModSeq,
    messages: Vec<TestMessage>,
}

impl TestMailbox {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            delimiter: '/',
            attributes: Vec::new(),
            subscribed: true,
            uid_validity: UidValidity::new(1),
            highest_mod_seq: ModSeq::new(1),
            messages: Vec::new(),
        }
    }

    pub fn delimiter(mut self, delimiter: char) -> Self {
        self.delimiter = delimiter;
        self
    }

    /// Leaving them off covers a server without `SPECIAL-USE`, which is the one Postio targets.
    pub fn attributes<I, S>(mut self, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.attributes = attributes.into_iter().map(Into::into).collect();
        self
    }

    pub fn subscribed(mut self, subscribed: bool) -> Self {
        self.subscribed = subscribed;
        self
    }

    pub fn uid_validity(mut self, uid_validity: UidValidity) -> Self {
        self.uid_validity = uid_validity;
        self
    }

    pub fn highest_mod_seq(mut self, highest_mod_seq: ModSeq) -> Self {
        self.highest_mod_seq = highest_mod_seq;
        self
    }

    pub fn message(mut self, message: TestMessage) -> Self {
        self.messages.push(message);
        self
    }

    pub fn corpus<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            self.messages.push(TestMessage::corpus(name.as_ref()));
        }
        self
    }

    fn seed(self) -> Mailbox {
        state::seed(
            self.path,
            self.delimiter,
            self.attributes,
            self.subscribed,
            self.uid_validity,
            self.highest_mod_seq,
            self.messages
                .into_iter()
                .map(|message| (message.raw, message.flags, message.internal_date))
                .collect(),
        )
    }
}

#[derive(Debug)]
pub(crate) struct Shared {
    state: Mutex<ServerState>,
    notify: Notify,
    stats: Arc<WireStats>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, ServerState> {
        self.state
            .lock()
            .expect("the test server state is poisoned")
    }

    fn notify(&self) {
        self.notify.notify_waiters();
    }

    fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.notify.notified()
    }
}

#[derive(Clone, Debug)]
pub struct TestServerBuilder {
    account: String,
    password: String,
    access_token: String,
    banner: Vec<String>,
    capabilities: Vec<String>,
    mailboxes: Vec<TestMailbox>,
    quirks: BTreeSet<Quirk>,
    idle_limit: Option<Duration>,
}

impl TestServerBuilder {
    fn new() -> Self {
        Self {
            account: DEFAULT_ACCOUNT.to_owned(),
            password: DEFAULT_PASSWORD.to_owned(),
            access_token: DEFAULT_ACCESS_TOKEN.to_owned(),
            banner: DEFAULT_BANNER
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            capabilities: DEFAULT_CAPABILITIES
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
            mailboxes: Vec::new(),
            quirks: BTreeSet::new(),
            idle_limit: None,
        }
    }

    pub fn account(mut self, account: impl Into<String>) -> Self {
        self.account = account.into();
        self
    }

    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = password.into();
        self
    }

    /// Advertising the mechanism is separate (`AUTH=OAUTHBEARER` in `capabilities`), so a test can
    /// describe a server that offers it and refuses every token (#193).
    pub fn access_token(mut self, token: impl Into<String>) -> Self {
        self.access_token = token.into();
        self
    }

    pub fn banner<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.banner = capabilities.into_iter().map(Into::into).collect();
        self
    }

    /// Everything is gated on this, so dropping a name covers the fallback for a server without it.
    pub fn capabilities<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.capabilities = capabilities.into_iter().map(Into::into).collect();
        self
    }

    pub fn mailbox(mut self, mailbox: TestMailbox) -> Self {
        self.mailboxes.push(mailbox);
        self
    }

    /// RFC 2177 §3 allows it after 29 minutes and middle-boxes are less patient; without this, a
    /// client that never re-arms looks fine.
    pub fn idle_limit(mut self, limit: Duration) -> Self {
        self.idle_limit = Some(limit);
        self
    }

    pub fn quirk(mut self, quirk: Quirk) -> Self {
        self.quirks.insert(quirk);
        self
    }

    pub async fn start(self) -> TestServer {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind a loopback port for the test server");
        let addr = listener.local_addr().expect("the bound address");

        let shared = Arc::new(Shared {
            state: Mutex::new(ServerState {
                account: self.account.clone(),
                password: self.password.clone(),
                access_token: self.access_token.clone(),
                banner: self.banner,
                capabilities: self.capabilities,
                mailboxes: self.mailboxes.into_iter().map(TestMailbox::seed).collect(),
                quirks: self.quirks,
                faults: Vec::new(),
                log: Vec::new(),
                idle_limit: self.idle_limit,
            }),
            notify: Notify::new(),
            stats: Arc::new(WireStats::default()),
        });

        let accepting = Arc::clone(&shared);
        let accept = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let shared = Arc::clone(&accepting);
                let stats = Arc::clone(&shared.stats);
                tokio::spawn(async move {
                    // A connection ending badly is usually the point — a
                    // dropped socket is a fault a test asked for.
                    let _ = session::Session::new(Conn::new(stream, stats), shared)
                        .run()
                        .await;
                });
            }
        });

        TestServer {
            addr,
            shared,
            accept,
            account: self.account,
            password: self.password,
            access_token: self.access_token,
        }
    }
}

#[derive(Debug)]
pub struct TestServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept: JoinHandle<()>,
    account: String,
    password: String,
    access_token: String,
}

impl TestServer {
    pub fn builder() -> TestServerBuilder {
        TestServerBuilder::new()
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub fn password(&self) -> &str {
        &self.password
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Cleartext, which `validate` allows on loopback only.
    #[cfg(feature = "imap")]
    pub fn settings(&self) -> crate::imap::ConnectionSettings {
        crate::imap::ConnectionSettings::new(
            self.addr.ip().to_string(),
            self.addr.port(),
            postio_model::TransportSecurity::None,
            self.account.clone(),
        )
    }

    /// Reaches the server through memory, for a paused clock (`start_paused`) or a connection that
    /// fills up: `capacity` is how many bytes the pipe holds before a write blocks.
    #[cfg(feature = "imap")]
    pub fn connector(&self, capacity: usize) -> InMemoryConnector {
        InMemoryConnector::new(Arc::clone(&self.shared), capacity)
    }

    pub fn inject(&self, fault: Fault) {
        self.shared.lock().faults.push(fault);
    }

    pub fn quirk(&self, quirk: Quirk) {
        self.shared.lock().quirks.insert(quirk);
    }

    pub fn compressed_connections(&self) -> usize {
        self.shared
            .stats
            .compressed_connections
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// `(protocol bytes, socket bytes)`; they differ only on connections that negotiated
    /// compression.
    pub fn bytes_sent(&self) -> (usize, usize) {
        let stats = &self.shared.stats;
        (
            stats.logical_out.load(std::sync::atomic::Ordering::SeqCst),
            stats.wire_out.load(std::sync::atomic::Ordering::SeqCst),
        )
    }

    pub fn commands(&self) -> Vec<String> {
        self.shared.lock().log.clone()
    }

    pub fn highest_mod_seq(&self, path: &str) -> u64 {
        self.with(path, |mailbox| mailbox.highest_mod_seq)
    }

    pub fn uid_validity(&self, path: &str) -> UidValidity {
        UidValidity::new(self.with(path, |mailbox| mailbox.uid_validity))
    }

    pub fn uids(&self, path: &str) -> Vec<Uid> {
        self.with(path, |mailbox| {
            mailbox.uids().into_iter().map(Uid::new).collect()
        })
    }

    pub fn flags(&self, path: &str, uid: Uid) -> FlagSet {
        self.with(path, |mailbox| {
            mailbox
                .find(uid.get())
                .map(|message| message.flags.clone())
                .unwrap_or_default()
        })
    }

    pub fn deliver(&self, path: &str, message: TestMessage) -> Uid {
        let uid = {
            let mut state = self.shared.lock();
            let mailbox = state
                .mailbox_mut(path)
                .unwrap_or_else(|| panic!("no mailbox at {path:?}"));
            mailbox.push(
                message.raw,
                message.flags,
                message.internal_date.unwrap_or_else(Utc::now),
            )
        };
        self.shared.notify();
        Uid::new(uid)
    }

    pub fn set_flags(&self, path: &str, uid: Uid, flags: FlagSet) {
        {
            let mut state = self.shared.lock();
            let mailbox = state
                .mailbox_mut(path)
                .unwrap_or_else(|| panic!("no mailbox at {path:?}"));
            let mod_seq = mailbox.bump();
            if let Some(message) = mailbox.find_mut(uid.get()) {
                message.flags = flags;
                message.mod_seq = mod_seq;
            }
        }
        self.shared.notify();
    }

    /// Remembered for `VANISHED (EARLIER)`, so a QRESYNC resync still reports it to a client that
    /// was away.
    pub fn vanish(&self, path: &str, uid: Uid) {
        {
            let mut state = self.shared.lock();
            let mailbox = state
                .mailbox_mut(path)
                .unwrap_or_else(|| panic!("no mailbox at {path:?}"));
            mailbox.remove(uid.get());
        }
        self.shared.notify();
    }

    pub fn set_uid_validity(&self, path: &str, uid_validity: UidValidity) {
        {
            let mut state = self.shared.lock();
            let mailbox = state
                .mailbox_mut(path)
                .unwrap_or_else(|| panic!("no mailbox at {path:?}"));
            mailbox.renumber(uid_validity.get());
        }
        self.shared.notify();
    }

    fn with<T>(&self, path: &str, read: impl FnOnce(&Mailbox) -> T) -> T {
        let state = self.shared.lock();
        let mailbox = state
            .mailbox(path)
            .unwrap_or_else(|| panic!("no mailbox at {path:?}"));
        read(mailbox)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.accept.abort();
    }
}
