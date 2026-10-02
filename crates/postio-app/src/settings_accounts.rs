//! The classic window's settings panel, joined to the store: the shared
//! presenters (`postio_widgets::present::settings`, T233) over this
//! window's panel, with the window answering what is the app's -- the
//! connection test and the token-expiry line from this process, a removal's
//! toast, a role mapping's command.

use std::rc::Rc;

use gtk::glib;
use postio_client::Client;
use postio_gtk::feed::Feeds;
use postio_gtk::window::Window;
use postio_model::Account;
use postio_model::ids::AccountId;
use postio_widgets::present::settings::{Later, Outside, Reached};

use crate::Wiring;
use crate::frontend::Frontend;

/// Accounts whose local search index is being rebuilt right now (#981):
/// the shared presenter's, read by [`crate::search`] to raise a search
/// outcome's corpus caveat while an account's index is mid-rebuild.
pub type Reindexing = postio_widgets::present::settings::Reindexing;

/// Wires `window`'s settings panel to `wiring`'s store.
pub async fn install(
    window: &Window,
    wiring: &Wiring,
    client: Client,
    reindexing: Reindexing,
    feeds: &Feeds,
) {
    install_for(
        window,
        &Frontend::over(wiring, client.clone()),
        client,
        reindexing,
        feeds,
    )
    .await;
}

/// [`install`], for a window whose store's owner may be another process.
pub async fn install_for(
    window: &Window,
    frontend: &Frontend,
    client: Client,
    reindexing: Reindexing,
    feeds: &Feeds,
) {
    let accounts = postio_widgets::present::settings::accounts::Accounts::install(
        &window.settings(),
        client,
        outside(window, frontend),
        reindexing,
    )
    .await;
    feeds.connect_event(move |event| accounts.hear(event));
}

/// Reads every account and redraws the panel's rows from it: after a
/// credential update, and when an account joins a running window.
pub(crate) async fn refresh(window: &Window, frontend: &Frontend, client: &Client) {
    postio_widgets::present::settings::accounts::Accounts::over(
        &window.settings(),
        client.clone(),
        outside(window, frontend),
        Reindexing::default(),
    )
    .refresh()
    .await;
}

fn outside(window: &Window, frontend: &Frontend) -> Rc<dyn Outside> {
    Rc::new(Classic {
        window: glib::object::ObjectExt::downgrade(window),
        frontend: frontend.clone(),
    })
}

/// What the classic window answers for its settings panel.
struct Classic {
    window: glib::WeakRef<Window>,
    frontend: Frontend,
}

impl Outside for Classic {
    fn test_connection(&self, account: Account) -> Later<Reached> {
        // The read and the two connections happen off the GTK loop, on this
        // process's runtime, and the answer comes back over a channel.
        let secrets = self.frontend.secrets.clone();
        let (sender, receiver) = async_channel::bounded(1);
        self.frontend.runtime.spawn(async move {
            let found = postio_session::reachability::test_over_tls(&account, &secrets).await;
            let _ = sender.send(found.into_results()).await;
        });
        Box::pin(async move {
            receiver.recv().await.unwrap_or_else(|_| {
                // The task died. Saying so beats the spinner that stops.
                let died = || Err("the test did not finish".to_owned());
                (died(), died())
            })
        })
    }

    fn token_expiries(
        &self,
        accounts: Vec<(AccountId, String)>,
    ) -> Later<Vec<(AccountId, Option<std::time::SystemTime>)>> {
        let secrets = self.frontend.secrets.clone();
        let (sender, receiver) = async_channel::bounded(1);
        self.frontend.runtime.spawn(async move {
            let expiries =
                postio_session::reachability::token_expiries(secrets.as_ref(), accounts).await;
            let _ = sender.send(expiries).await;
        });
        Box::pin(async move { receiver.recv().await.unwrap_or_default() })
    }

    fn update_credential(&self, account: AccountId, saved: Box<dyn Fn()>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let frontend = self.frontend.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: reading the account is a client call, a
            // oneshot receive; the host answers on its own runtime.
            crate::settings_credential::open(&window, &frontend, account, saved).await;
        });
    }

    fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>) {
        if let Some(window) = self.window.upgrade() {
            window.show_removable_toast(description, undo);
        }
    }

    fn run(&self, command: postio_core::Command) {
        if let Some(window) = self.window.upgrade() {
            window.act(command);
        }
    }

    fn attachments_eager(&self) -> bool {
        self.frontend.attachments_eager
    }
}
