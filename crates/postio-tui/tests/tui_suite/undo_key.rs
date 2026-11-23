//! `ctrl+z` undoes in the terminal: raw mode delivers undo's `mod+z` as a key, and nothing
//! suspends the process.

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use postio_core::bridge::event_channel;
use postio_core::state::SharedState;
use postio_core::{Command, CommandId, MessageTarget};
use postio_model::{MailboxId, Message, MessageId};
use postio_session::actions::Actions;
use postio_storage::repository::MessageRepository;
use postio_storage::{Store, test_support};
use postio_tui::input::Keys;
use postio_ui::keymap::{KeyContext, Outcome};

async fn folder_of(database: &Store, message: MessageId) -> MailboxId {
    let connection = database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .get(message)
        .await
        .expect("a read")
        .expect("the message is still there")
        .mailbox_id
}

#[tokio::test]
async fn after_an_archive_ctrl_z_puts_the_message_back() {
    let database = test_support::memory().await;
    let (inbox, message) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        test_support::mailbox(&connection, &account, "Archive").await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        (inbox, id)
    };
    let actions = Actions::new(database.clone(), SharedState::default());
    let (sink, _events) = event_channel();

    actions
        .run(
            &Command::Archive {
                target: MessageTarget::Messages(vec![message]),
            },
            &sink,
        )
        .await
        .expect("the archive runs");
    assert_ne!(
        folder_of(&database, message).await,
        inbox,
        "the archive did not take the message out of the inbox, so nothing \
         below can show an undo"
    );

    let (mut keys, problems) = Keys::new(&postio_core::Keymap::for_terminal(&Default::default()));
    assert!(problems.is_empty(), "{problems:?}");
    let pressed = keys.press(
        &KeyEvent {
            code: KeyCode::Char('z'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
        KeyContext::List,
        false,
    );
    let Outcome::Command(id) = pressed else {
        panic!("ctrl+z runs nothing in the terminal: {pressed:?}");
    };
    let command = Command::default_for(id.parse::<CommandId>().expect("a built-in command"));
    actions
        .run(&command, &sink)
        .await
        .expect("the command ctrl+z resolved to runs");

    assert_eq!(
        folder_of(&database, message).await,
        inbox,
        "ctrl+z ran `{id}`, and the archived message is not back in the inbox"
    );
}
