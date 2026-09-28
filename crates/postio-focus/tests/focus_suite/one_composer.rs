//! Focus has no composer of its own (US3 scenario 2, FR-050; T081): the
//! same content written in Focus and in the classic app leaves as the same
//! message, byte for byte.
//!
//! The classic app's side is its composer -- `postio_widgets::composer::
//! Composer`, the one widget both apps mount -- on a host of this test's own,
//! sending as `postio-app`'s `install_send` does: the draft the composer
//! hands over, queued through a `ClientKind::Gtk` client of the same host.
//! Focus may not depend on `postio-app` even for a test, so its seam is
//! restated here; what is compared is what the host queued for each, built
//! into the message the drainer would send.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_core::{CommandId, Keymap};
use postio_model::listing::{ListPage, MailStore as _, PageRequest};
use postio_model::{Draft, Identity, ListScope, MessageId};
use postio_widgets::composer::{Composer, ComposerHost};

use crate::support::{self, Fixture};

/// Who hears the commands a host dispatches.
type Handlers = RefCell<Vec<Box<dyn Fn(CommandId)>>>;

/// A host with nothing of either app in it: a window and a box.
struct Bare {
    window: gtk::Window,
    pane: gtk::Box,
    commands: Handlers,
}

impl ComposerHost for Bare {
    fn parent(&self) -> Option<gtk::Window> {
        Some(self.window.clone())
    }
    fn install(&self, composer: &Composer) {
        composer.set_visible(false);
        self.pane.append(composer);
    }
    fn restore(&self, composer: &Composer) {
        self.pane.append(composer);
    }
    fn remove(&self, composer: &Composer) {
        self.pane.remove(composer);
    }
    fn take_pane(&self) {}
    fn release_pane(&self) {}
    fn command_for_key(
        &self,
        _key: gdk::Key,
        _state: gdk::ModifierType,
        _window: &gtk::Window,
    ) -> Option<CommandId> {
        None
    }
    fn handle_key(
        &self,
        _key: gdk::Key,
        _state: gdk::ModifierType,
        _window: &gtk::Window,
    ) -> glib::Propagation {
        glib::Propagation::Proceed
    }
    fn add_action(&self, _action: &gtk::gio::SimpleAction) {}
    fn connect_command(&self, handler: Box<dyn Fn(CommandId)>) {
        self.commands.borrow_mut().push(handler);
    }
    fn keymap(&self) -> Keymap {
        Keymap::defaults().clone()
    }
    fn composing(&self, _open: bool, _keymap: &Keymap) {}
    fn adopt(&self, _window: &gtk::Window) {}
}

/// The Outbox's rows, as the classic app lists them.
async fn outbox(classic: &Client, fixture: &Fixture) -> Vec<MessageId> {
    match classic
        .list_page(PageRequest {
            scope: ListScope::Outbox(fixture.account.id),
            offset: 0,
            limit: 20,
        })
        .await
    {
        Ok(ListPage::Messages(page)) => page.rows.into_iter().map(|row| row.id).collect(),
        Ok(ListPage::Threads(page)) => page
            .rows
            .into_iter()
            .map(|row| row.representative.id)
            .collect(),
        Err(error) => panic!("the Outbox could not be listed: {error}"),
    }
}

/// The same message, written into `composer`: a recipient, a subject and a
/// line of body.
fn write(composer: &Composer) {
    composer.test_set_to("Lena Park <lena@example.org>, ");
    composer.test_set_subject("Harbor API draft v3");
    composer.test_set_body("v3 looks good. Two comments on the rate-limit headers.");
}

/// The message the drainer would send for `draft`, with the two lines that
/// differ between any two sends -- when, and which Message-ID -- set aside.
fn leaving(draft: &Draft, identity: &Identity) -> String {
    let built = postio_model::outgoing::build(draft, identity, &[], None, None);
    String::from_utf8_lossy(&built.raw)
        .lines()
        .filter(|line| !line.starts_with("Date:") && !line.starts_with("Message-ID:"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn the_same_content_from_either_app_leaves_as_the_same_message() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        {
            let connection = fixture.database.connect().await.expect("a connection");
            postio_storage::test_support::mailbox(&connection, &fixture.account, "Drafts").await;
        }
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let window = postio_focus::window::FocusWindow::new(None);
        window.present();
        let session = postio_focus::startup::adopt(
            &window,
            fixture.host(),
            &postio_config::Config::default(),
        );
        let classic = session.host().connect(ClientKind::Gtk);
        support::keep(session);
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "Focus mounted no composer"
        );

        // Focus writes it, and sends with its key.
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "c opened no composer"
        );
        let dialog = window.compose_dialog().expect("the compose dialog");
        assert_eq!(
            support::with_class(&dialog, "postio-composer").len(),
            1,
            "the dialog holds the one composer both apps mount, and no other"
        );
        let focus_composer = window.composer().expect("Focus's composer");
        assert!(
            focus_composer.is_ancestor(&dialog),
            "the composer on screen is the classic app's widget"
        );
        write(&focus_composer);
        support::press(&window, "Return", gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_none()).await,
            "Ctrl+Return did not send: {}",
            focus_composer.status()
        );
        assert!(
            crate::settle_until(async || outbox(&classic, &fixture).await.len() == 1).await,
            "Focus's message never reached the Outbox"
        );
        let from_focus = outbox(&classic, &fixture).await[0];

        // The classic app writes the same, and sends as its seam does.
        let host_window = gtk::Window::new();
        host_window.set_default_size(900, 700);
        let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        host_window.set_child(Some(&pane));
        host_window.present();
        let classic_composer = Composer::new();
        classic_composer.mount_on(Rc::new(Bare {
            window: host_window.clone(),
            pane,
            commands: RefCell::default(),
        }));
        classic_composer.set_account(fixture.account.id);
        classic_composer.connect_send({
            let classic = classic.clone();
            let composer = classic_composer.downgrade();
            move |draft| {
                if let Some(composer) = composer.upgrade() {
                    drop(classic.queue_send(composer.generation(), draft.clone(), None));
                }
            }
        });
        classic_composer.dispatch(CommandId::Compose);
        write(&classic_composer);
        classic_composer.dispatch(CommandId::Send);
        assert!(
            crate::settle_until(async || outbox(&classic, &fixture).await.len() == 2).await,
            "the classic app's message never reached the Outbox"
        );
        let from_classic = *outbox(&classic, &fixture)
            .await
            .iter()
            .find(|row| **row != from_focus)
            .expect("the classic app's row");

        let identity = Identity::new(fixture.account.id, fixture.account.address.clone());
        let focus_draft = classic
            .draft_behind(from_focus)
            .await
            .expect("a read")
            .expect("Focus's queued draft");
        let classic_draft = classic
            .draft_behind(from_classic)
            .await
            .expect("a read")
            .expect("the classic app's queued draft");
        let focus_bytes = leaving(&focus_draft, &identity);
        // Not two empty messages that happen to agree: each line written is
        // in what leaves.
        for written in [
            "Subject: Harbor API draft v3",
            "To: \"Lena Park\" <lena@example.org>",
            "v3 looks good. Two comments on the rate-limit headers.",
        ] {
            assert!(
                focus_bytes.contains(written),
                "{written:?} is not in what leaves:\n{focus_bytes}"
            );
        }
        assert_eq!(
            focus_bytes,
            leaving(&classic_draft, &identity),
            "the same content leaves as the same message from either app"
        );
    });
}
