//! A store with mail in it, a host over it, and a server over the host.
//!
//! The mailbox: Ada's account, `test@example.com`, with an inbox and an
//! archive, and a second account nobody shares. Every address is on a
//! reserved domain and nothing reaches the network: the host runs no engine.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use chrono::Utc;
use postio_account::secret::MemorySecretStore;
use postio_host::Host;
use postio_mcp::{Audit, Backend, Held, Lease, Server};
use postio_model::{AccountId, EmailAddress, MailboxId, Message, MessageId};
use postio_storage::Store;
use postio_storage::repository::{
    AccountRepository, MessageRepository, OperationQueueRepository, StoredBody, ThreadingRepository,
};
use postio_storage::test_support;
use serde_json::{Value, json};

/// The address of the account a test shares.
pub const ADA: &str = "test@example.com";
/// The address of the account no test shares.
pub const OTHER: &str = "other@example.org";

/// `[mcp]` sharing Ada's inbox and nothing else.
pub const INBOX_ONLY: &str = "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"INBOX\"]\n";
/// `[mcp]` sharing every folder of Ada's account.
pub const EVERY_FOLDER: &str = "[mcp.accounts.\"test@example.com\"]\nmailboxes = [\"*\"]\n";

/// The words every message here is found by, so a search can reach them all.
pub const COMMON: &str = "harborview";

/// A backend that counts how often the store was asked for.
pub struct Counted {
    held: Held,
    leases: Rc<Cell<u32>>,
}

impl Backend for Counted {
    fn lease(&mut self) -> Result<Lease, String> {
        self.leases.set(self.leases.get() + 1);
        self.held.lease()
    }
}

/// The world a case runs in.
pub struct World {
    pub rt: tokio::runtime::Runtime,
    pub database: Store,
    pub ada: AccountId,
    pub other: AccountId,
    pub inbox: MailboxId,
    /// In the inbox: "Atlas budget", from Grace.
    pub budget: MessageId,
    /// In the archive: "Quarterly numbers".
    pub archived: MessageId,
    /// In the inbox: HTML with a script and a remote image.
    pub html: MessageId,
    /// In the other account's inbox.
    pub elsewhere: MessageId,
    pub server: Server<Counted>,
    pub leases: Rc<Cell<u32>>,
    pub config: PathBuf,
    pub audit: PathBuf,
    host: Arc<Host>,
    next_id: Cell<u64>,
    _directory: tempfile::TempDir,
}

impl World {
    /// The mailbox, with `[mcp]` as `shared` says.
    pub fn new(shared: &str) -> World {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let directory = tempfile::tempdir().expect("a directory");
        let (database, ada, other, inbox, archive, other_inbox) = rt.block_on(async {
            let database = test_support::memory().await;
            let connection = database.connect().await.expect("a connection");
            postio_index::index::ensure_schema(&connection)
                .await
                .expect("the index");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            let archive = test_support::mailbox(&connection, &account, "Archive").await;
            test_support::mailbox(&connection, &account, "Drafts").await;
            let mut other =
                postio_model::Account::new("Other", EmailAddress::new(Some("Other Person"), OTHER));
            other.incoming.host = "imap.example.org".to_owned();
            other.outgoing.host = "smtp.example.org".to_owned();
            AccountRepository::new(&connection)
                .create(&mut other)
                .await
                .expect("a second account");
            let other_inbox = test_support::mailbox(&connection, &other, "INBOX").await;
            (
                database,
                account.id,
                other.id,
                inbox,
                archive.id,
                other_inbox.id,
            )
        });

        let blobs = postio_storage::BlobStore::open(
            directory.path().join("blobs"),
            &test_support::blob_keys(),
        )
        .expect("a blob store");
        let host = Arc::new(
            Host::start(database.clone(), blobs, |wiring| {
                wiring.with_secrets(Arc::new(MemorySecretStore::new()))
            })
            .expect("a host"),
        );

        let config = directory.path().join("config.toml");
        std::fs::write(&config, shared).expect("the config");
        let audit = directory.path().join("mcp-access.log");
        let leases = Rc::new(Cell::new(0));
        let server = Server::new(
            Counted {
                held: Held::new(Arc::clone(&host)),
                leases: Rc::clone(&leases),
            },
            Some(config.clone()),
            Audit::to_file(audit.clone()),
        );

        let mut world = World {
            rt,
            database,
            ada,
            other,
            inbox,
            budget: MessageId::UNASSIGNED,
            archived: MessageId::UNASSIGNED,
            html: MessageId::UNASSIGNED,
            elsewhere: MessageId::UNASSIGNED,
            server,
            leases,
            config,
            audit,
            host,
            next_id: Cell::new(1),
            _directory: directory,
        };
        world.budget = world.message(ada, inbox, |message| {
            message.subject = Some("Atlas budget".into());
            message.from = vec![EmailAddress::new(Some("Grace Hopper"), "grace@example.org")];
            message.to = vec![EmailAddress::new(Some("Test User"), ADA)];
            message.body.text = Some(format!("The atlas budget is due Friday. {COMMON}\n\nGrace"));
        });
        world.archived = world.message(ada, archive, |message| {
            message.subject = Some("Quarterly numbers".into());
            message.from = vec![EmailAddress::new(None::<&str>, "finance@example.net")];
            message.body.text = Some(format!("The quarterly numbers are in. {COMMON}"));
        });
        world.html = world.message(ada, inbox, |message| {
            message.subject = Some("Newsletter".into());
            message.from = vec![EmailAddress::new(None::<&str>, "news@example.net")];
            message.body.text = None;
            message.body.html = Some(format!(
                "<p>Hello <b>reader</b>, {COMMON}.</p>\
                 <script>steal(document.cookie)</script>\
                 <img src=\"https://tracker.example.net/pixel.gif\" width=1 height=1>\
                 <p onclick=\"steal()\">Read on.</p>"
            ));
        });
        world.elsewhere = world.message(other, other_inbox, |message| {
            message.subject = Some("Private matter".into());
            message.from = vec![EmailAddress::new(None::<&str>, "someone@example.net")];
            message.body.text = Some(format!("Not for the agent. {COMMON}"));
        });
        world
    }

    /// A message in `mailbox`, its body stored and indexed the way the
    /// backfill leaves one.
    pub fn message(
        &self,
        account: AccountId,
        mailbox: MailboxId,
        shape: impl FnOnce(&mut Message),
    ) -> MessageId {
        let mut message = Message::new(account, mailbox, Utc::now());
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(format!(
            "mcp-suite-{}@example.invalid",
            self.next_id.replace(self.next_id.get() + 1)
        )));
        shape(&mut message);
        self.store(message)
    }

    /// `message`, written with its body and indexed.
    pub fn store(&self, mut message: Message) -> MessageId {
        message.sync.body_state = postio_model::BodyState::Full;
        let body = message.body.clone();
        self.rt.block_on(async {
            let connection = self.database.connect().await.expect("a connection");
            let messages = MessageRepository::new(&connection);
            let id = messages.create(&mut message).await.expect("a message");
            messages
                .set_body(
                    id,
                    &StoredBody {
                        text: body.text.clone(),
                        html: body.html.clone(),
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::BodyState::Full,
                )
                .await
                .expect("the body");
            postio_index::index::index_body_of(&connection, id.get(), &body)
                .await
                .expect("indexed");
            message.id = id;
            ThreadingRepository::new(&connection, message.account_id)
                .thread(&message)
                .await
                .expect("threaded");
            id
        })
    }

    /// Change what `[mcp]` shares, as a person editing the file would.
    pub fn share(&self, shared: &str) {
        std::fs::write(&self.config, shared).expect("the config");
    }

    /// Send one JSON-RPC request and read its answer.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id.replace(self.next_id.get() + 1);
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let answer = self
            .server
            .handle(&line.to_string())
            .expect("a request is answered");
        let answer: Value = serde_json::from_str(&answer).expect("the answer is JSON");
        assert_eq!(answer["jsonrpc"], "2.0", "{answer}");
        assert_eq!(answer["id"], json!(id), "{answer}");
        answer
    }

    /// Call `tool` with `arguments`, and read its result.
    pub fn call(&mut self, tool: &str, arguments: Value) -> ToolResult {
        let answer = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        assert!(answer.get("error").is_none(), "a protocol error: {answer}");
        ToolResult(answer["result"].clone())
    }

    /// Every operation waiting to reach a server, in any account.
    pub fn queued(&self) -> Vec<postio_model::Operation> {
        self.rt.block_on(async {
            let connection = self.database.connect().await.expect("a connection");
            let queue = OperationQueueRepository::new(&connection);
            let far = Utc::now() + chrono::Duration::days(3650);
            let mut all = Vec::new();
            for account in [self.ada, self.other] {
                all.extend(
                    queue
                        .pending(account, far)
                        .await
                        .expect("the queue")
                        .into_iter()
                        .map(|queued| queued.operation),
                );
            }
            all
        })
    }

    /// Every draft in every account.
    pub fn drafts(&self) -> Vec<postio_model::Draft> {
        self.rt.block_on(async {
            let connection = self.database.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection);
            let mut all = drafts.list_for_account(self.ada).await.expect("drafts");
            all.extend(drafts.list_for_account(self.other).await.expect("drafts"));
            all
        })
    }

    /// The access log's lines.
    pub fn audit_lines(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.audit)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line is JSON"))
            .collect()
    }

    /// The host, for a case that asks it something directly.
    pub fn host(&self) -> &Host {
        &self.host
    }
}

/// A `tools/call` result.
pub struct ToolResult(pub Value);

impl ToolResult {
    /// Whether the tool said it could not do it.
    pub fn is_error(&self) -> bool {
        self.0["isError"].as_bool().unwrap_or(false)
    }

    /// The server's own words: the first block.
    pub fn notice(&self) -> String {
        self.0["content"][0]["text"]
            .as_str()
            .expect("a text block")
            .to_owned()
    }

    /// The mail the tool returned, parsed: the second block, under its
    /// untrusted label. Panics when there is none.
    pub fn mail(&self) -> Value {
        let text = self.0["content"][1]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("no mail block: {}", self.0));
        let block: Value = serde_json::from_str(text).expect("the mail block is JSON");
        block["untrusted_mail_content"].clone()
    }

    /// Everything the result says, as one string, for "never says X".
    pub fn all_text(&self) -> String {
        self.0.to_string()
    }
}
