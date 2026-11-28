use chrono::Duration;
use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, MailboxId};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::test_support;

use crate::conversations::{Mail, file, now, store, today};

async fn found(
    connection: &postio_storage::Checkout,
    mailboxes: Option<&[MailboxId]>,
) -> (Vec<postio_model::MessageId>, u64) {
    let query = parse("harbour", today());
    let request = SearchRequest {
        account: AccountScope::Unified,
        query: &query,
        scope: Scope::AllMail,
        mailboxes,
        limit: 50,
        order: postio_search::ResultOrder::Newest,
    };
    let results = search(connection, &request, now()).await.expect("search");
    (
        results.hits.iter().map(|hit| hit.message_id).collect(),
        results.total_hits,
    )
}

#[tokio::test]
async fn a_folder_limit_keeps_other_folders_out_of_the_hits_and_the_total() {
    let (_database, connection, account, inbox) = store().await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let mail = |days| Mail {
        subject: "Harbour plan",
        ago: Duration::days(days),
        ..Mail::default()
    };
    let shared = file(&connection, &account, inbox, mail(3)).await;
    file(&connection, &account, archive, mail(2)).await;
    file(&connection, &account, archive, mail(1)).await;

    assert_eq!(
        found(&connection, None).await.1,
        3,
        "unlimited, every folder"
    );
    assert_eq!(
        found(&connection, Some(&[inbox])).await,
        (vec![shared.id], 1),
        "the total counts only the folders given, or a count would say what the others hold"
    );
    assert_eq!(
        found(&connection, Some(&[])).await,
        (Vec::new(), 0),
        "no folder given is nothing, never everything"
    );
}
