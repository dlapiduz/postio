use std::time::{Duration, Instant};

use postio_client::protocol::ClientKind;
use postio_host::{FocusSetup, Host};
use postio_model::ListScope;
use postio_model::listing::{MailStore, PageRequest};

const BUDGET: Duration = Duration::from_millis(500);

const MESSAGES: usize = 5_000;

const FIRST_PAGE: u32 = 60;

async fn first_frame(client: &postio_client::Client) -> usize {
    let accounts = client.accounts().await.expect("the accounts");
    assert!(
        accounts.iter().any(|account| account.enabled),
        "an enabled account"
    );
    let scope = ListScope::Focus(postio_model::FocusScope::Inbox);
    let total = client.list_count(scope).await.expect("the inbox's count");
    client.surfaced().await.expect("the surfaced rows");
    let page = client
        .list_page(PageRequest {
            scope,
            offset: 0,
            limit: FIRST_PAGE,
        })
        .await
        .expect("the first page");
    let rows = match page {
        postio_model::listing::ListPage::Messages(page) => page.rows.len(),
        postio_model::listing::ListPage::Threads(page) => {
            let threads = page.rows.iter().filter_map(|row| row.id).collect();
            client.thread_labels(threads).await.expect("the labels");
            page.rows.len()
        }
    };
    client.focus_counts().await.expect("the strip's counts");
    assert!(
        u64::from(total) >= rows as u64,
        "a count that holds the page"
    );
    rows
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
    let focus = host.enable_focus(FocusSetup::from_config(Default::default(), None));
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
    drop(focus);
    host.stop();
}
