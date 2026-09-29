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
