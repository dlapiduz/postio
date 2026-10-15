//! A call across the boundary runs on whatever thread the caller is on.
//!
//! Swift calls the exported methods from `Task.detached`, which runs on the
//! cooperative pool, and a thread there has a **512 KiB** stack rather than
//! the main thread's 8 MiB. The engine's deepest work is compiling a write:
//! Turso builds every trigger and foreign-key cascade the statement can
//! fire into its program at prepare time, recursively, and a debug build's
//! async frames sit under that. The Mac app died of it on launch, with
//! SIGBUS in a guard page, for any demo seed that queues a send.
//!
//! How much of that 512 KiB is left by the time Swift calls in is not
//! something this side can know, so the contract is that it does not
//! matter: these call the exported, blocking entry points the way Swift
//! does, from a thread with a stack smaller than the work needs, and look at
//! what the store holds afterwards.

use chrono::Utc;
use postio_ffi::{Session, SessionOptions};
use postio_model::{DraftState, EmailAddress, Message};
use postio_storage::repository::{DraftRepository, MessageRepository};
use postio_storage::test_support;

/// Less than a send needs (about 230 KiB in a test build), and a quarter of
/// what a Swift cooperative-pool thread starts with.
const CALLER_STACK: usize = 128 * 1024;

/// Runs `call` on a foreign thread with a small stack, as Swift would.
fn from_a_swift_task<T: Send + 'static>(call: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .name("swift-cooperative".to_owned())
        .stack_size(CALLER_STACK)
        .spawn(call)
        .expect("a thread")
        .join()
        .expect("the call returns rather than panicking")
}

fn store_with_an_account() -> postio_storage::Store {
    tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(async {
            let database = test_support::memory().await;
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.from = vec![EmailAddress::new(Some("Ada Norwood"), "ada@example.com")];
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            database
        })
}

#[test]
fn sending_from_a_small_foreign_stack_queues_the_draft() {
    let database = store_with_an_account();
    let session = Session::open(SessionOptions::in_memory_with(database.clone()))
        .expect("a session over the store");

    let complaint = from_a_swift_task({
        let session = session.clone();
        move || {
            let mut draft = session.new_draft_ffi().expect("an account to write from");
            draft.to = "bo@example.com".to_owned();
            draft.subject = "The gate".to_owned();
            draft.body = "Six is fine.".to_owned();
            session.send_draft_ffi(draft)
        }
    });
    assert_eq!(complaint, None, "the send is queued without a complaint");

    let queued = tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(async {
            let connection = database.connect().await.expect("a connection");
            DraftRepository::new(&connection)
                .list_for_account(postio_model::ids::AccountId::new(1))
                .await
                .expect("a list")
        });
    assert_eq!(queued.len(), 1, "one draft, queued");
    assert_eq!(queued[0].state, DraftState::Queued);
    session.shutdown();
}
