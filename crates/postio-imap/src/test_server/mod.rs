//! An in-process IMAP server, for tests that need the wire.
//!
//! # Why this exists
//!
//! Postio had two test doubles and neither one touched the protocol.
//! [`MockBackend`](crate::backend::MockBackend) sits at the `MailBackend`
//! trait, so everything above it is well covered and `io-imap` is never
//! exercised at all. [`ImapScript`](crate::imap::ImapScript) replays a fixed
//! transcript, which proved the capability-banner case but cannot answer a
//! command sequence nobody wrote down, and cannot model state that changes.
//!
//! `io-imap` is pre-1.0 and shipped six minor releases in eleven weeks
//! (ADR 0001), which makes it the layer most likely to regress under us and
//! the one nothing was watching. So this is a *stateful* server the real
//! client stack can talk to over a loopback socket: real bytes, real
//! `io-imap`, real session and auth path, in an ordinary `cargo test` with no
//! network.
//!
//! ```no_run
//! # use postio_imap::test_server::{TestMailbox, TestServer};
//! # async fn example() {
//! let server = TestServer::builder()
//!     .mailbox(TestMailbox::new("INBOX").corpus(["plain-text-simple"]))
//!     .start()
//!     .await;
//!
//! // Point a ConnectionPool at `server.settings()` and sync it.
//! # }
//! ```
//!
//! # Fault injection is the point
//!
//! A server that only behaves correctly would test half of what matters. The
//! failures Postio has to survive are the ones a real provider actually
//! produces: extensions hidden until after login, a missing `* ENABLED` echo,
//! a `FETCH` sequence number no decoder can read, a connection that dies
//! halfway through a body. Two of those can silently lose mail. See [`Quirk`]
//! for the ones a server *is*, and [`Fault`] for the ones it *does* once.
//!
//! # What it is not
//!
//! A complete IMAP server. It implements what Postio sends plus enough
//! around the edges to be useful, and answers anything else with `BAD`
//! rather than guessing — a test server that improvised would let a client's
//! mistake through. There is no TLS: plaintext on loopback is what
//! [`ConnectionSettings::validate`](crate::imap::ConnectionSettings::validate)
//! allows for exactly this, and a certificate would add nothing a test can
//! observe.

mod mime;
mod session;
mod state;
mod wire;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use postio_model::{FlagSet, ModSeq, Uid, UidValidity};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use self::state::{Mailbox, ServerState};
use self::wire::Conn;

/// The account the server accepts, unless a builder says otherwise.
const DEFAULT_ACCOUNT: &str = "someone@example.com";

/// The password it accepts. An app-specific password, in spirit.
const DEFAULT_PASSWORD: &str = "app-specific-password";

/// What the greeting advertises by default: nothing worth having.
///
/// The provider Postio targets hides `CONDSTORE`, `QRESYNC`, `IDLE` and
/// `UIDPLUS` until after login, so that is what the default server does. A
/// client that trusts the banner degrades to full resync forever without one
/// error being logged, and this is the shape of server that proves it does
/// not — ADR 0001, Q3.
const DEFAULT_BANNER: [&str; 4] = ["IMAP4rev1", "SASL-IR", "AUTH=PLAIN", "AUTH=LOGIN"];

/// What `CAPABILITY` reports once authenticated.
const DEFAULT_CAPABILITIES: [&str; 13] = [
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
];

/// A way this server misbehaves for as long as it is set.
///
/// Every one of these is something a mainstream provider has actually done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Quirk {
    /// Advertise the full capability set in the greeting, before login.
    ///
    /// The *conformant* behaviour, and not the default: hiding extensions
    /// until after authentication is what the provider Postio targets does,
    /// and is the case worth being wrong about.
    AdvertiseExtensionsBeforeLogin,

    /// Put `[CAPABILITY …]` in the tagged OK that ends authentication.
    ///
    /// Saves the client a round trip. Without it — the default — the client
    /// has to ask again, which is the path `ensure_capabilities` exists for
    /// and the one every Postio session must take.
    CapabilityInLoginResponse,

    /// Answer `ENABLE` with a bare tagged OK and no untagged `* ENABLED`.
    ///
    /// A violation of RFC 5161 §3.1 that at least one mainstream provider has
    /// shipped. A client that gates QRESYNC on the echo rather than on the
    /// post-auth capability list silently loses incremental sync.
    OmitEnabledEcho,

    /// Emit `* -1 FETCH (…)` during a `CHANGEDSINCE` pull.
    ///
    /// A sequence number is a `NonZeroU32`, so the line cannot be decoded.
    /// `io-imap` skips an undecodable untagged response and completes the
    /// command `Ok`, which means a resync that lost a message's flags looks
    /// exactly like one that did not.
    MalformedFetchSequenceNumber,
}

/// Something the server does once, to the next command that matches.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// Answer part of the response and hang up.
    ///
    /// Against `FETCH` this tears the connection in the middle of a body
    /// literal, which is the case that matters: the client has already been
    /// told how many octets are coming.
    DropConnection {
        /// The command name to fire on, e.g. `"FETCH"`.
        during: String,
    },
    /// Accept the command and never answer it.
    Stall {
        /// The command name to fire on.
        during: String,
    },
    /// Answer `NO`.
    Reject {
        /// The command name to fire on.
        during: String,
        /// The text after `NO`.
        reason: String,
    },
}

impl Fault {
    fn matches(&self, command: &str) -> bool {
        let during = match self {
            Self::DropConnection { during }
            | Self::Stall { during }
            | Self::Reject { during, .. } => during,
        };
        during.eq_ignore_ascii_case(command)
    }
}

/// A message to seed a mailbox with.
#[derive(Clone, Debug)]
pub struct TestMessage {
    raw: Vec<u8>,
    flags: FlagSet,
    internal_date: Option<DateTime<Utc>>,
}

impl TestMessage {
    /// A message from its raw RFC 5322 bytes.
    pub fn new(raw: impl Into<Vec<u8>>) -> Self {
        Self {
            raw: raw.into(),
            flags: FlagSet::new(),
            internal_date: None,
        }
    }

    /// A message from the shared `.eml` corpus, by fixture name.
    ///
    /// The corpus is the one every other crate tests against, so a sync test
    /// and a parser test are looking at the same bytes.
    pub fn corpus(name: &str) -> Self {
        Self::new(postio_model::test_corpus::load(name).bytes().to_vec())
    }

    /// Sets the flags the message already carries.
    pub fn with_flags(mut self, flags: FlagSet) -> Self {
        self.flags = flags;
        self
    }

    /// Sets `INTERNALDATE`; otherwise it comes from the `Date` header.
    pub fn with_internal_date(mut self, at: DateTime<Utc>) -> Self {
        self.internal_date = Some(at);
        self
    }
}

/// A mailbox to seed the server with.
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
    /// An empty mailbox at `path`.
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

    /// Sets the hierarchy delimiter the server reports.
    pub fn delimiter(mut self, delimiter: char) -> Self {
        self.delimiter = delimiter;
        self
    }

    /// Sets the `LIST` attributes, e.g. `\Sent`.
    ///
    /// Leaving them off is how a test covers a server without `SPECIAL-USE`,
    /// which is the one Postio targets.
    pub fn attributes<I, S>(mut self, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.attributes = attributes.into_iter().map(Into::into).collect();
        self
    }

    /// Sets whether the account is subscribed to this mailbox.
    pub fn subscribed(mut self, subscribed: bool) -> Self {
        self.subscribed = subscribed;
        self
    }

    /// Sets the mailbox's UID generation.
    pub fn uid_validity(mut self, uid_validity: UidValidity) -> Self {
        self.uid_validity = uid_validity;
        self
    }

    /// Sets the modification sequence the mailbox starts at.
    pub fn highest_mod_seq(mut self, highest_mod_seq: ModSeq) -> Self {
        self.highest_mod_seq = highest_mod_seq;
        self
    }

    /// Adds a message. UIDs are assigned in insertion order from 1.
    pub fn message(mut self, message: TestMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Adds messages from the `.eml` corpus, by fixture name.
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

/// The state every connection shares, and the bell they ring on a change.
#[derive(Debug)]
pub(crate) struct Shared {
    state: Mutex<ServerState>,
    notify: Notify,
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

/// Builds a [`TestServer`].
#[derive(Clone, Debug)]
pub struct TestServerBuilder {
    account: String,
    password: String,
    banner: Vec<String>,
    capabilities: Vec<String>,
    mailboxes: Vec<TestMailbox>,
    quirks: BTreeSet<Quirk>,
}

impl TestServerBuilder {
    fn new() -> Self {
        Self {
            account: DEFAULT_ACCOUNT.to_owned(),
            password: DEFAULT_PASSWORD.to_owned(),
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
        }
    }

    /// Sets the account the server accepts.
    pub fn account(mut self, account: impl Into<String>) -> Self {
        self.account = account.into();
        self
    }

    /// Sets the password the server accepts.
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = password.into();
        self
    }

    /// Sets what the greeting advertises, before login.
    pub fn banner<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.banner = capabilities.into_iter().map(Into::into).collect();
        self
    }

    /// Sets what `CAPABILITY` reports once authenticated.
    ///
    /// This is what every extension is gated on, so dropping a name here is
    /// how a test covers the fallback path for a server without it.
    pub fn capabilities<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.capabilities = capabilities.into_iter().map(Into::into).collect();
        self
    }

    /// Adds a mailbox.
    pub fn mailbox(mut self, mailbox: TestMailbox) -> Self {
        self.mailboxes.push(mailbox);
        self
    }

    /// Makes the server misbehave in a particular way from the start.
    pub fn quirk(mut self, quirk: Quirk) -> Self {
        self.quirks.insert(quirk);
        self
    }

    /// Binds an ephemeral loopback port and starts serving.
    pub async fn start(self) -> TestServer {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind a loopback port for the test server");
        let addr = listener.local_addr().expect("the bound address");

        let shared = Arc::new(Shared {
            state: Mutex::new(ServerState {
                account: self.account.clone(),
                password: self.password.clone(),
                banner: self.banner,
                capabilities: self.capabilities,
                mailboxes: self.mailboxes.into_iter().map(TestMailbox::seed).collect(),
                quirks: self.quirks,
                faults: Vec::new(),
                log: Vec::new(),
            }),
            notify: Notify::new(),
        });

        let accepting = Arc::clone(&shared);
        let accept = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let shared = Arc::clone(&accepting);
                tokio::spawn(async move {
                    // A connection ending badly is usually the point — a
                    // dropped socket is a fault a test asked for.
                    let _ = session::Session::new(Conn::new(stream), shared).run().await;
                });
            }
        });

        TestServer {
            addr,
            shared,
            accept,
            account: self.account,
            password: self.password,
        }
    }
}

/// A running IMAP server on a loopback port.
///
/// Dropping it stops accepting connections; open ones end with the test.
#[derive(Debug)]
pub struct TestServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept: JoinHandle<()>,
    account: String,
    password: String,
}

impl TestServer {
    /// Starts building a server.
    pub fn builder() -> TestServerBuilder {
        TestServerBuilder::new()
    }

    /// Where it is listening.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The account it accepts.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// The password it accepts.
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Settings that reach it: cleartext, because
    /// [`validate`](crate::imap::ConnectionSettings::validate) allows that on
    /// loopback and nowhere else.
    #[cfg(feature = "imap")]
    pub fn settings(&self) -> crate::imap::ConnectionSettings {
        crate::imap::ConnectionSettings::new(
            self.addr.ip().to_string(),
            self.addr.port(),
            postio_model::TransportSecurity::None,
            self.account.clone(),
        )
    }

    /// Arms a fault. The next matching command fires it, once.
    pub fn inject(&self, fault: Fault) {
        self.shared.lock().faults.push(fault);
    }

    /// Makes the server misbehave from now on.
    pub fn quirk(&self, quirk: Quirk) {
        self.shared.lock().quirks.insert(quirk);
    }

    /// Every command line the server has received, tags included.
    pub fn commands(&self) -> Vec<String> {
        self.shared.lock().log.clone()
    }

    /// The mailbox's current `HIGHESTMODSEQ`.
    pub fn highest_mod_seq(&self, path: &str) -> u64 {
        self.with(path, |mailbox| mailbox.highest_mod_seq)
    }

    /// The mailbox's current `UIDVALIDITY`.
    pub fn uid_validity(&self, path: &str) -> UidValidity {
        UidValidity::new(self.with(path, |mailbox| mailbox.uid_validity))
    }

    /// The UIDs the mailbox holds, in order.
    pub fn uids(&self, path: &str) -> Vec<Uid> {
        self.with(path, |mailbox| {
            mailbox.uids().into_iter().map(Uid::new).collect()
        })
    }

    /// One message's flags.
    pub fn flags(&self, path: &str, uid: Uid) -> FlagSet {
        self.with(path, |mailbox| {
            mailbox
                .find(uid.get())
                .map(|message| message.flags.clone())
                .unwrap_or_default()
        })
    }

    /// Delivers a message, as new mail arriving would.
    ///
    /// Wakes any connection sitting in `IDLE`.
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

    /// Sets one message's flags, as another client would.
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

    /// Removes a message, as another client's expunge would.
    ///
    /// It is remembered for `VANISHED (EARLIER)`, so a QRESYNC resync can
    /// still report it to a client that was away.
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

    /// Renumbers the mailbox's UID space, as a restore from backup does.
    ///
    /// Every UID a client holds becomes meaningless, which is the event the
    /// sync engine has to notice rather than paper over.
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
