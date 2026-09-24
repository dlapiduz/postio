//! Adding another account to a running application (#64, ADR 0012).
//!
//! The first-run screen knows how to collect an account and knows how to run
//! exactly once: [`crate::onboarding::install`] replaces the window's whole
//! content, which is right when there is nothing behind it and wrong when
//! there is a mailbox on screen. ADR 0012 Q1 settled that with *one form,
//! two hosts* — the widget, its states, its probe-on-commit timing and its
//! `connect_probe`/`connect_submit` seam are untouched here, which is the
//! test of whether the reuse was real. [`crate::settings_credential`] is the
//! third host, and this module is deliberately shaped like it.
//!
//! # Why a command rather than a button
//!
//! `docs/ARCHITECTURE.md` §2: a command that is not in the registry does not
//! exist. [`CommandId::AddAccount`] is what puts *Add account* in the
//! palette, the `?` cheat sheet and `docs/keybindings.md` at once, which is
//! where a keyboard-first user looks before they open a settings panel. The
//! settings panel deliberately grows no second button of its own, for the
//! reason `postio_gtk::settings`'s own module docs give about
//! `CommandId::EditConfig`: the palette already gives it an accessible
//! control, and two entry points that must agree forever is the cost of the
//! duplicate.
//!
//! # Closing the dialogue cancels the probe
//!
//! ADR 0012 Q3. This is the first surface where a probe runs a second time
//! in one process, over a shell that stays on screen, with a way for the
//! user to walk away from it — so a discovery request that outlives its
//! dialogue is a socket held open for an answer that lands on a form no
//! longer in the tree (#57). `AdwDialog::connect_closed` covers every way
//! out: `Esc`, the close button, and the dialogue being dismissed by its
//! parent going away.
//!
//! # What it does *not* do
//!
//! Give the new account a place in the sidebar. That is still keyed to one
//! account and is #1's work; [`crate::attach_account`] is the seam it will
//! land on.

use gtk::glib;
use std::sync::Arc;

use adw::prelude::*;
use postio_account::discovery::{DiscoveryTransport, PimalayaTransport};
use postio_client::Client;
use postio_core::CommandId;
use postio_gtk::onboarding::Onboarding;
use postio_gtk::window::Window;

use crate::Wiring;
use crate::frontend::Frontend;
use crate::onboarding::{JmapOfferSlot, ProbeCancellation, probe, submit};

/// Wire [`CommandId::AddAccount`] to the dialogue.
///
/// Through `connect_command` rather than the command bus: the bus answers
/// verbs over mail, and this one is answered by the composition root, which
/// is the only place that may build a probe and write an account row.
pub async fn install(window: &Window, wiring: &Wiring) {
    install_for(window, &Frontend::in_process(wiring)).await;
}

/// [`install`], for a window whose store's owner may be another process.
pub async fn install_for(window: &Window, frontend: &Frontend) {
    // Weak: this handler is stored on the window itself, so a strong clone
    // is a cycle with no third party in it at all (#1072).
    let weak = glib::object::ObjectExt::downgrade(window);
    window.connect_command({
        let frontend = frontend.clone();
        move |id| {
            postio_session::blocking::now(async {
                if id == CommandId::AddAccount {
                    let Some(window) = weak.upgrade() else {
                        return;
                    };
                    // Built per opening, not once: a transport is cheap, and one
                    // shared between dialogues would outlive the cancellation
                    // that is supposed to end its work.
                    open_for(
                        &window,
                        &frontend,
                        // Discovery probes are outbound connections too (#151).
                        Arc::new(PimalayaTransport::new().with_egress(frontend.egress.clone())),
                    )
                    .await;
                }
            })
        }
    });
}

/// Opens the add-account dialogue over `window` and hands back the dialogue,
/// so a caller — and a test — can close it the way the user would.
///
/// `transport` is supplied rather than constructed, for the reason #282 gave
/// [`crate::onboarding::install`]: a probe that builds its own transport can
/// only be reached by dialling the network, and no test in the default suite
/// may.
pub async fn open(
    window: &Window,
    wiring: &Wiring,
    transport: Arc<dyn DiscoveryTransport>,
) -> adw::Dialog {
    // The write, and reading its row back, are the store owner's (ADR 0041),
    // asked through a client of this dialogue's own over the same wiring.
    open_for(window, &Frontend::in_process(wiring), transport).await
}

/// [`open`], for a window whose store's owner may be another process.
pub async fn open_for(
    window: &Window,
    frontend: &Frontend,
    transport: Arc<dyn DiscoveryTransport>,
) -> adw::Dialog {
    let screen = Onboarding::new();
    // A fresh form starts at its first field, which is the name.
    screen.focus_name();

    let dialog = adw::Dialog::builder()
        .title("Add account")
        .content_width(420)
        .content_height(420)
        .child(&screen)
        .build();

    let cancellation = ProbeCancellation::default();
    // Every way out of the dialogue, including the ones with no button:
    // `Esc`, the close gesture, and the parent window closing under it.
    dialog.connect_closed({
        let cancellation = cancellation.clone();
        move |_| cancellation.stop()
    });

    let client = frontend.client.clone();

    let jmap = JmapOfferSlot::default();
    screen.connect_probe({
        let screen = screen.clone();
        let runtime = frontend.runtime.clone();
        let cancellation = cancellation.clone();
        let jmap = jmap.clone();
        move |address| {
            probe(
                &screen,
                &runtime,
                address,
                &cancellation,
                Arc::clone(&transport),
                jmap.clone(),
            )
        }
    });

    screen.connect_submit({
        let screen = screen.clone();
        let runtime = frontend.runtime.clone();
        let cancellation = cancellation.clone();
        let on_saved = {
            let window = window.clone();
            let frontend = frontend.clone();
            let dialog = dialog.clone();
            let client = client.clone();
            move |address: &str| {
                postio_session::blocking::now(async {
                    dialog.close();
                    join(&window, &frontend, &client, address).await;
                })
            }
        };
        move |submission| {
            // Pressing Connect settles the question the probe was asking,
            // and the dialogue is on its way out either way.
            cancellation.stop();
            let address = submission.address.clone();
            let on_saved = on_saved.clone();
            submit(
                &screen,
                &runtime,
                &client,
                submission.clone(),
                jmap.borrow().clone(),
                move || on_saved(&address),
            )
        }
    });

    dialog.present(Some(window));
    dialog
}

/// Bring the account just written at `address` into the running application.
///
/// Read back from the store rather than built from the submission: the row
/// is what [`crate::attach_account`] has to start an engine from, it carries
/// the id only the insert knows, and `onboarding::save` may have *updated*
/// an account that was already there rather than creating one.
async fn join(window: &Window, frontend: &Frontend, client: &Client, address: &str) {
    let Some(account) = written(client, address).await else {
        // The row was written a moment ago, so this is a store that has
        // stopped answering — which the panes are about to say far more
        // loudly than a toast would.
        tracing::error!("the account was saved and could not be read back");
        return;
    };
    let Some(wiring) = &frontend.wiring else {
        // The owner is another process, which starts the new account's
        // sync itself when asked; a refusal is its log's to say.
        client.start_sync();
        crate::settings_accounts::refresh(window, frontend, client).await;
        return;
    };
    if let Err(refusal) = crate::attach_account(window, wiring, &account).await {
        // The account exists and is enabled; what it has not got is an
        // engine. Said on screen rather than only logged, because the
        // sentence names the two things the user can do about it and
        // nothing else in the window is going to explain why the account
        // they just added is not syncing.
        tracing::error!(%refusal, "the account was added but is not syncing: {refusal}");
        window.show_action_completed(&refusal.to_string(), false);
    }
}

/// The account row for `address`, however it was written.
async fn written(client: &Client, address: &str) -> Option<postio_model::Account> {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    client
        .accounts()
        .await
        .ok()?
        .into_iter()
        .find(|account| account.address.address.eq_ignore_ascii_case(address))
}
