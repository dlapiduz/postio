//! What a frontend's first usable frame waits on, against the constitution's
//! 500 ms (specs/005-tui-frontend SC-003, T094).
//!
//! POSTIO-MEASUREMENT: it builds a store of five thousand messages, and its
//! output is numbers a person reads, so it runs nightly (`.config/nextest.toml`
//! excludes it from the default profile). Run it by hand with
//! `cargo nextest run -p postio-host --profile nightly --test startup_budget --no-capture`.
//!
//! An app draws its first usable frame once it has opened the store in its
//! own process -- the file, with the schema and search-index checks
//! `open_store_at` makes -- started the host over it, connected its client,
//! and read through that client the accounts, the account's folders and the
//! inbox's first page: what `postio-tui`'s `run` and `first_scope` and its
//! first `Open` do, and what the desktop app's `open_the_store` does before
//! its list reads.
//!
//! Two waits are deliberately in neither number. The keyring round trip is a
//! D-Bus call to a Secret Service this measurement cannot assume is there; the
//! desktop's startup timeline measures it where it happens. Drawing the frame
//! is `postio-tui`'s pure code over rows already in hand, microseconds beside
//! these.

use std::time::{Duration, Instant};

use postio_client::protocol::ClientKind;
use postio_host::Host;
use postio_model::listing::{MailStore, PageRequest};
use postio_model::mailbox::MailboxRole;
use postio_model::{ListScope, MailboxId};

/// SC-003: the first usable frame within this.
const BUDGET: Duration = Duration::from_millis(500);

/// A mailbox big enough that a read which scaled with it would show.
const MESSAGES: usize = 5_000;

/// The first page a terminal of ordinary height asks for.
const FIRST_PAGE: u32 = 60;

/// What the first frame reads, over `client`: the accounts, the first
/// account's folders, and its inbox's first page.
async fn first_frame(client: &postio_client::Client) -> usize {
    let accounts = client.accounts().await.expect("the accounts");
    let account = accounts
        .iter()
        .find(|account| account.enabled)
        .expect("an enabled account");
    let folders = client.mailboxes(account.id).await.expect("the folders");
    let inbox: MailboxId = folders
        .iter()
        .find(|folder| folder.role == MailboxRole::Inbox)
        .expect("an inbox")
        .id;
    let page = client
        .list_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 0,
            limit: FIRST_PAGE,
        })
        .await
        .expect("the first page");
    match page {
        postio_model::listing::ListPage::Messages(page) => page.rows.len(),
        postio_model::listing::ListPage::Threads(page) => page.rows.len(),
    }
}

#[test]
fn an_app_reaches_its_first_usable_frame_within_the_budget() {
    let store_dir = tempfile::tempdir().expect("a store directory");
    let path = store_dir.path().join("postio.db");
    let key = postio_storage::key::StoreKey::generate();
    let open = || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("a runtime to open the store on")
            .block_on(postio_session::open_store_at(&path, &key))
            .expect("the store opens")
    };

    // The mailbox, written once and closed, so the open below reads a store
    // that exists rather than creating one.
    {
        let (database, _blobs) = open();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime to seed on")
            .block_on(postio_storage::seed::seed_large(&database, 7, MESSAGES));
    }

    let start = Instant::now();
    let (database, blobs) = open();
    let host = Host::start(database, blobs, |wiring| wiring).expect("a host");
    let client = host.connect(ClientKind::Tui);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a frontend's runtime");
    let rows = rt.block_on(first_frame(&client));
    let took = start.elapsed();
    assert_eq!(rows, FIRST_PAGE as usize, "a first page to draw");

    println!(
        "first usable frame over {MESSAGES} messages: {} ms (budget {} ms)",
        took.as_millis(),
        BUDGET.as_millis()
    );
    assert!(took < BUDGET, "a start took {took:?}");

    drop(client);
    host.stop();
}
