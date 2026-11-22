use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use adw::prelude::*;
use gtk::glib;
use postio_client::api::Call;
use postio_client::protocol::{AfterSave, Req, Resp};
use postio_client::{Client, Transport};
use postio_core::EventEnvelope;
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress};
use postio_widgets::onboarding::{Onboarding, Status};

use crate::support::until;

struct Scripted {
    account: Account,
    asked: Mutex<Vec<Req>>,
    events: async_channel::Receiver<EventEnvelope>,
}

impl Transport for Scripted {
    fn call(&self, request: Req) -> Call<'static> {
        let answer = match &request {
            Req::Accounts => Resp::Accounts(vec![self.account.clone()]),
            _ => Resp::Done,
        };
        self.asked.lock().expect("never poisoned").push(request);
        Box::pin(async move { Ok(answer) })
    }

    fn post(&self, request: Req) {
        self.asked.lock().expect("never poisoned").push(request);
    }

    fn events(&self) -> async_channel::Receiver<EventEnvelope> {
        self.events.clone()
    }
}

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

pub fn the_credential_dialog_reads_the_account_and_saves_through_the_client() {
    if gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let mut account = Account::new(
        "Ada Moreno",
        EmailAddress::new(None::<String>, "ada@example.com"),
    );
    account.id = AccountId::new(7);
    account.incoming.host = "imap.example.com".to_owned();
    account.outgoing.host = "smtp.example.com".to_owned();
    let (_tell, events) = async_channel::unbounded();
    let host = Arc::new(Scripted {
        account,
        asked: Mutex::new(Vec::new()),
        events,
    });
    let client = Client::new(host.clone());

    let window = adw::Window::new();
    window.set_default_size(800, 600);
    window.present();

    let dialog: Rc<RefCell<Option<adw::Dialog>>> = Rc::default();
    let saved = Rc::new(Cell::new(0u32));
    glib::MainContext::default().spawn_local({
        let window = window.clone();
        let dialog = dialog.clone();
        let saved = saved.clone();
        async move {
            let opened = postio_widgets::present::onboarding::update_credential(
                &window,
                &client,
                |account| account.id == AccountId::new(7),
                |_| panic!("a password update opened a browser"),
                move || saved.set(saved.get() + 1),
            )
            .await;
            dialog.replace(opened);
        }
    });
    assert!(
        until(|| dialog.borrow().is_some()),
        "no credential dialog for an account the client knows"
    );
    let shown = dialog.borrow().clone().expect("the dialog");
    let closed = Rc::new(Cell::new(false));
    shown.connect_closed({
        let closed = closed.clone();
        move |_| closed.set(true)
    });
    let form = form_in(shown.upcast_ref()).expect("the account form in the dialog");

    assert_eq!(form.address(), "ada@example.com");
    let Status::Reauthenticate(settings) = form.status() else {
        panic!("the form did not arrive as a repair: {:?}", form.status());
    };
    assert_eq!(settings.imap.host, "imap.example.com");
    assert_eq!(settings.smtp.host, "smtp.example.com");

    form.test_set_password("a new app password");
    form.submit();
    assert!(
        until(|| saved.get() > 0),
        "the new password was never saved: {:?}",
        form.status()
    );
    assert_eq!(saved.get(), 1, "saved once");
    assert!(
        matches!(form.status(), Status::Saved),
        "{:?}",
        form.status()
    );
    assert!(
        until(|| closed.get()),
        "the dialog stayed open after the save"
    );

    let asked = host.asked.lock().expect("never poisoned");
    let adds: Vec<_> = asked
        .iter()
        .filter_map(|request| match request {
            Req::AddAccount { submission, then } => Some((submission.clone(), *then)),
            _ => None,
        })
        .collect();
    assert_eq!(adds.len(), 1, "one save asked for: {asked:?}");
    let (submission, then) = &adds[0];
    assert_eq!(submission.address, "ada@example.com");
    assert_eq!(submission.password, "a new app password");
    assert_eq!(submission.settings.imap.host, "imap.example.com");
    assert_eq!(
        *then,
        AfterSave::Wait,
        "a credential update started a second sync for an account already syncing"
    );
}
