//! T171: Focus's first run, over a store with no account at all -- the
//! shared add-account form (`postio_widgets::present::onboarding::add_account`,
//! T165) opens over the window by itself, and saving through it mounts what
//! an account being known mounts, brings its connection up, and leaves the
//! inbox listing (the maintainer's T149 walk found no such path at all).
//!
//! T172: `c` and the top bar's compose button, asked while there is still no
//! account -- the wizard dismissed rather than finished -- say what is
//! missing and offer the same form, rather than doing nothing.

use adw::prelude::*;
use postio_account::backend::MockBackend;
use postio_core::CommandId;
use postio_model::TransportSecurity;
use postio_widgets::onboarding::{Onboarding, Server, Settings, Status};

use crate::support::{self, NoAccount};

/// The account form anywhere under `widget`: the dialog `add_account`
/// presents.
fn form_in(widget: &gtk::Widget) -> Option<Onboarding> {
    if let Ok(form) = widget.clone().downcast::<Onboarding>() {
        return Some(form);
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(form) = form_in(&current) {
            return Some(form);
        }
        child = current.next_sibling();
    }
    None
}

/// Manually entered settings, as a person who typed a domain and edited the
/// server fields by hand would arrive at -- no probe, so no network.
fn typed_settings(address: &str) -> Settings {
    Settings {
        imap: Server {
            host: "imap.example.test".to_owned(),
            port: 993,
            security: TransportSecurity::Tls,
        },
        smtp: Server {
            host: "smtp.example.test".to_owned(),
            port: 465,
            security: TransportSecurity::Tls,
        },
        login: address.to_owned(),
        ..Settings::default()
    }
}

/// Fills `form` as a person adding `address` by hand would, and presses
/// Connect.
fn add_through(form: &Onboarding, name: &str, address: &str, password: &str) {
    form.set_name(name);
    form.set_address(address);
    form.set_status(Status::Found(typed_settings(address)));
    form.test_set_password(password);
    form.submit();
}

/// A window over a store with no account, its host proving any account
/// added against `backend` rather than a real network.
async fn opened(backend: MockBackend) -> (postio_focus::window::FocusWindow, MockBackend) {
    let store = NoAccount::new().await;
    let window = postio_focus::window::FocusWindow::new(None);
    window.present();
    let session = postio_focus::startup::adopt(
        &window,
        store.host_signing_in_to(backend.clone()),
        &postio_config::Config::default(),
    );
    support::keep(session);
    support::keep(store);
    (window, backend)
}

pub fn first_run_with_no_account_opens_the_add_account_form_and_lists_the_inbox() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let backend = MockBackend::new();
        let (window, backend) = opened(backend).await;

        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "an empty store opened no add-account form"
        );
        let dialog = window.add_account_dialog().expect("the wizard");
        let form = form_in(dialog.upcast_ref()).expect("the account form in the dialog");

        let calls_before_save = backend.calls();
        add_through(
            &form,
            "Grace Okafor",
            "grace@example.test",
            "an app password",
        );

        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "the wizard stayed open after the save: {:?}",
            form.status()
        );

        // What an account being known mounts (`Self::mount_compose`): the
        // composer, once the accounts refresh the save kicked off has landed.
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "no composer was mounted for the account just saved"
        );
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "compose still did nothing once an account existed"
        );
        window.compose_dialog().expect("a composer").close();

        // Its connection comes up: a second call to the backend, beyond the
        // one that proved the credential.
        assert!(
            crate::settle_until(async || backend.calls() > calls_before_save + 1).await,
            "the new account's sync never started"
        );
    });
}

pub fn compose_with_no_account_says_so_and_offers_to_add_one() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let backend = MockBackend::new();
        let (window, backend) = opened(backend).await;

        // The maintainer's scenario: the wizard came up on its own (T171)
        // and was dismissed without finishing it, the way `Esc` or the
        // dialog's own close button would leave it.
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "an empty store opened no add-account form"
        );
        window.add_account_dialog().expect("the wizard").close();
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "the wizard did not close"
        );

        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.toast_showing().is_some()).await,
            "c did nothing at all with no account"
        );
        assert_eq!(
            window.toast_showing().as_deref(),
            Some("There's no account to write from yet."),
            "compose with no account should say what is missing"
        );
        assert_eq!(
            window.toast().expect("the toast").button_label().as_deref(),
            Some("Add account"),
            "the toast offers no way to add one"
        );

        // The action the toast's button names: the same command reopens the
        // form.
        window.act(CommandId::AddAccount);
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "CommandId::AddAccount opened no form"
        );
        let dialog = window.add_account_dialog().expect("the wizard");
        let form = form_in(dialog.upcast_ref()).expect("the account form in the dialog");
        add_through(
            &form,
            "Grace Okafor",
            "grace@example.test",
            "an app password",
        );
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "the wizard stayed open after the save: {:?}",
            form.status()
        );

        // Once an account exists, compose works as now (T172): once the
        // accounts refresh the save kicked off has landed.
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "no composer was mounted for the account just saved"
        );
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "compose still did nothing once an account existed"
        );
        let _ = backend;
    });
}

/// The scrolled body of the account form: the pane the server fields would
/// have to be scrolled in.
fn body_scroller(form: &Onboarding) -> gtk::ScrolledWindow {
    support::descendants(form)
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
        .expect("the form's scrolled body")
}

/// What the account form with its server fields and a refusal showing needs,
/// with room to spare for a larger font.
const FORM_HEIGHT: i32 = 600;

/// T174: the form opens large enough that the mail-server details, once
/// shown, fit without scrolling.
pub fn the_wizard_opens_large_enough_for_the_server_details() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _backend) = opened(MockBackend::new()).await;
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "an empty store opened no add-account form"
        );
        let dialog = window.add_account_dialog().expect("the wizard");
        let form = form_in(dialog.upcast_ref()).expect("the account form in the dialog");
        form.set_status(Status::Found(typed_settings("grace@example.test")));
        form.show_manual(true);
        // As it stands after a first attempt: the settings found and a
        // refusal to read.
        form.test_set_password("an app password");
        form.set_status(Status::Failed(
            "The server refused the password.".to_owned(),
        ));
        form.show_manual(true);
        crate::settle();
        let scroller = body_scroller(&form);
        let adjustment = scroller.vadjustment();
        assert!(
            crate::settle_until(async || adjustment.page_size() > 0.0).await,
            "the form was never laid out"
        );
        assert!(
            dialog.content_height() >= FORM_HEIGHT,
            "the form opens {}px tall, less than the {FORM_HEIGHT}px its details want",
            dialog.content_height()
        );
        assert!(
            adjustment.upper() <= adjustment.page_size() + 1.0,
            "the server details need scrolling: {} of {} px show",
            adjustment.page_size(),
            adjustment.upper()
        );
    });
}

/// T174: the window's close button still closes the app while the form is
/// open. The form's dialog covers the window and takes a click meant for it,
/// so the click has to be the window's before the dialog sees it.
pub fn the_window_close_button_closes_the_app_with_the_form_open() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (window, _backend) = opened(MockBackend::new()).await;
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "an empty store opened no add-account form"
        );
        crate::settle();
        let close = support::only(&window, "focus-close");
        let bounds = close
            .compute_bounds(&window)
            .expect("the close button has a place in the window");
        let (x, y) = (
            f64::from(bounds.x() + bounds.width() / 2.0),
            f64::from(bounds.y() + bounds.height() / 2.0),
        );
        assert!(window.click_through_dialog(x, y), "the click was not taken");
        assert!(
            crate::settle_until(async || !window.is_visible()).await,
            "the window's close button did nothing with the form open"
        );
    });
}

/// A message of the first sync's, as the server's mailbox holds it.
fn arriving(subject: &str, minutes: i64) -> postio_account::backend::MockMessage {
    let date = (chrono::Utc::now() - chrono::Duration::minutes(minutes)).to_rfc2822();
    postio_account::backend::MockMessage::new(format!(
        "From: Ada Moreno <ada@example.com>\r\nTo: grace@example.test\r\n\
         Subject: {subject}\r\nDate: {date}\r\nMessage-ID: <{subject}@example.com>\r\n\r\n\
         A line about {subject}.\r\n"
    ))
}

/// T178: the list's keys work in the state the maintainer's first run left
/// it in -- the account just added through the form, the first sync filling
/// the inbox -- not only on a store that was full from the start. `x` took
/// no row and the bulk bar never showed, which is the whole of selecting.
pub fn the_list_keys_work_while_the_first_sync_fills_the_inbox() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let backend = MockBackend::builder()
            .mailbox(
                postio_account::backend::MockMailbox::new("INBOX")
                    .message(arriving("First", 30))
                    .message(arriving("Second", 20))
                    .message(arriving("Third", 10)),
            )
            .build();
        let (window, _backend) = opened(backend).await;
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_some()).await,
            "an empty store opened no add-account form"
        );
        let dialog = window.add_account_dialog().expect("the wizard");
        let form = form_in(dialog.upcast_ref()).expect("the account form in the dialog");
        // A person types into the form: the keyboard is in its fields.
        form.focus_password();
        crate::settle();
        add_through(
            &form,
            "Grace Okafor",
            "grace@example.test",
            "an app password",
        );
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "the wizard stayed open after the save: {:?}",
            form.status()
        );
        crate::settle();
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 3).await,
            "the first sync's mail never reached the list: {:?}",
            support::subjects(&window)
        );

        support::keys(&window, &["j", "x"]);
        let bar = support::only(&window, "focus-bulk-bar");
        assert!(
            crate::settle_until(async || bar.is_mapped()).await,
            "x selected nothing: the bulk bar never showed"
        );
        assert!(
            support::texts(&bar).iter().any(|text| text == "1 selected"),
            "the bar counts one: {:?}",
            support::texts(&bar)
        );
        support::keys(&window, &["a"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "a archived nothing: {:?}",
            support::subjects(&window)
        );
    });
}
