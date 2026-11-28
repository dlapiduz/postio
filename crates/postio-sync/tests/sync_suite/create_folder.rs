use chrono::{DateTime, TimeZone, Utc};
use postio_account::backend::{Fault, MailBackend, MockBackend, MockMailbox};
use postio_model::{
    AccountId, Mailbox, MailboxId, MailboxRole, Message, Operation, OperationTarget,
};
use postio_storage::Connection;
use postio_storage::repository::{MailboxRepository, MessageRepository, OperationQueueRepository};
use postio_storage::test_support;
use postio_sync::Drainer;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 28, hour, 0, 0).unwrap()
}

async fn server() -> MockBackend {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX"))
        .build();
    backend.connect().await.expect("connect");
    backend
}

struct Local {
    account: AccountId,
    inbox: MailboxId,
    folder: MailboxId,
}

async fn a_pending_folder(connection: &Connection, path: &str) -> Local {
    let account = test_support::account(connection).await;
    let inbox = test_support::mailbox(connection, &account, "INBOX")
        .await
        .id;
    let mut folder = Mailbox::new(account.id, path, Some('/'));
    folder.role = MailboxRole::Regular;
    folder.pending_create = true;
    let folder = MailboxRepository::new(connection)
        .create(&mut folder)
        .await
        .expect("the pending folder");
    OperationQueueRepository::new(connection)
        .enqueue(
            account.id,
            OperationTarget::Mailbox(folder),
            &Operation::CreateMailbox { mailbox: folder },
            at(9),
        )
        .await
        .expect("enqueue the create");
    Local {
        account: account.id,
        inbox,
        folder,
    }
}

async fn pending(connection: &Connection, folder: MailboxId) -> bool {
    MailboxRepository::new(connection)
        .get(folder)
        .await
        .expect("read the folder")
        .expect("the folder is still here")
        .pending_create
}

#[tokio::test]
async fn a_pending_folder_is_created_on_the_server_and_stops_being_pending() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = a_pending_folder(&connection, "Projects/Harbour").await;
    let backend = server().await;

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.applied, 1, "{report:?}");
    assert_eq!(backend.created(), vec!["Projects/Harbour".to_owned()]);
    assert!(
        !pending(&connection, local.folder).await,
        "the server has it now"
    );
    assert!(
        report.mailboxes_changed,
        "the folder list changed, so the sidebar must hear it"
    );
}

#[tokio::test]
async fn a_refused_create_is_said_and_the_folder_stays_pending() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = a_pending_folder(&connection, "Receipts").await;
    let backend = server().await;
    backend.refuse_creates("folder quota reached");

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert_eq!(report.failed.len(), 1, "{report:?}");
    let said = report.failed[0].said();
    assert!(
        said.starts_with("Folder not created") && said.contains("folder quota reached"),
        "what did not happen, then the server's own why: {said}"
    );
    assert!(pending(&connection, local.folder).await);
}

#[tokio::test]
async fn a_move_into_a_folder_still_being_created_waits_instead_of_failing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let local = a_pending_folder(&connection, "Receipts").await;
    let backend = server().await;
    backend.inject_after(1, Fault::Io("network is unreachable".to_owned()));

    let mut message = Message::new(local.account, local.inbox, at(8));
    message.server.remote_id = Some(postio_model::RemoteId::new("1:1"));
    let message = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("a message");
    OperationQueueRepository::new(&connection)
        .enqueue(
            local.account,
            OperationTarget::Message(message),
            &Operation::Move {
                from: local.inbox,
                to: local.folder,
            },
            at(9),
        )
        .await
        .expect("enqueue the move");

    let report = Drainer::new(&backend)
        .drain(&connection, local.account, at(10))
        .await
        .expect("drain");

    assert!(
        report.failed.is_empty(),
        "a move whose destination is not on the server yet is not a lost destination: {report:?}"
    );
    assert_eq!(
        report.deferred, 2,
        "the create retries, and the move waits for it"
    );
}
