use adw::prelude::*;
use postio_account::backend::MockBackend;
use postio_model::TransportSecurity;
use postio_widgets::onboarding::{Onboarding, Server, Settings, Status, SyncWindow};

use crate::support::{self, NoAccount};

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

fn add_through(form: &Onboarding, name: &str, address: &str, password: &str) {
    form.set_name(name);
    form.set_address(address);
    form.set_status(Status::Found(typed_settings(address)));
    form.test_set_password(password);
    form.submit();
}

async fn start_sync(window: &postio_gtk::window::FocusWindow, form: &Onboarding) {
    assert!(
        crate::settle_until(async || matches!(form.status(), Status::SyncWindow)).await,
        "the form never asked how much history to sync: {:?}",
        form.status()
    );
    crate::settle();
    let start = support::button_labelled(window, "Start sync");
    support::click(window, &start, 1);
}

async fn opened(backend: MockBackend) -> (postio_gtk::window::FocusWindow, MockBackend) {
    let store = NoAccount::new().await;
    let window = postio_gtk::window::FocusWindow::new(None);
    window.present();
    let session = postio_gtk::startup::adopt(
        &window,
        store.host_signing_in_to(backend.clone()),
        &postio_config::Config::default(),
    );
    support::keep(session);
    support::keep(store);
    (window, backend)
}

fn config_contents(path: &std::path::Path) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("could not read the config: {error}"),
    }
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
        start_sync(&window, &form).await;
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "the wizard stayed open after the save: {:?}",
            form.status()
        );

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

        assert!(
            crate::settle_until(async || backend.calls() > calls_before_save + 1).await,
            "the new account's sync never started"
        );
    });
}

pub fn the_first_run_asks_how_much_history_to_sync_after_the_account_is_saved() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let config_path = postio_config::paths::config_path().expect("a config path");
        let config_before = config_contents(&config_path);
        let (window, backend) = opened(MockBackend::new()).await;
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
            crate::settle_until(async || matches!(form.status(), Status::SyncWindow)).await,
            "the saved account was not followed by the history step: {:?}",
            form.status()
        );
        crate::settle();
        assert!(
            window.add_account_dialog().is_some(),
            "the form closed before the history was chosen"
        );
        assert_eq!(
            config_contents(&config_path),
            config_before,
            "the history was written before it was chosen"
        );
        assert_eq!(
            backend.calls(),
            calls_before_save + 1,
            "the account's sync started before the history was chosen"
        );

        form.test_select_sync_window(SyncWindow::LastMonth);
        let start = support::button_labelled(&window, "Start sync");
        support::click(&window, &start, 1);

        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "Start sync left the form open"
        );
        let written = std::fs::read_to_string(&config_path).expect("config.toml was written");
        let config = postio_config::Config::from_toml_str(&written).expect("it still parses");
        assert_eq!(
            config.sync.initial_sync_messages,
            SyncWindow::LastMonth.message_count(),
            "the chosen history did not reach [sync].initial_sync_messages: {written}"
        );
        assert!(
            crate::settle_until(async || backend.calls() > calls_before_save + 1).await,
            "the account's sync never started once the history was chosen"
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

        let add = support::button_labelled(&window, "Add account");
        support::click(&window, &add, 1);
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
        start_sync(&window, &form).await;
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "the wizard stayed open after the save: {:?}",
            form.status()
        );

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

fn body_scroller(form: &Onboarding) -> gtk::ScrolledWindow {
    support::descendants(form)
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
        .expect("the form's scrolled body")
}

fn connect_button(form: &Onboarding) -> gtk::Button {
    support::descendants(form)
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Button>().ok())
        .find(|button| {
            support::descendants(button).into_iter().any(|widget| {
                widget
                    .downcast::<gtk::Label>()
                    .is_ok_and(|label| label.text() == "Connect")
            })
        })
        .expect("the form's Connect button")
}

/// Waits for the server fields to be laid out before measuring: earlier, the body has the height of
/// the form without them and the case passes whatever the dialog's size.
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
        form.test_set_password("an app password");
        form.set_status(Status::Failed(
            "The server refused the password.".to_owned(),
        ));
        form.show_manual(true);
        let scroller = body_scroller(&form);
        let body = scroller
            .child()
            .and_then(|viewport| viewport.first_child())
            .expect("the form's body");
        let connect = connect_button(&form);
        let adjustment = scroller.vadjustment();
        let laid_out = || {
            let width = body.width();
            if width == 0 || !connect.is_mapped() || connect.height() == 0 {
                return false;
            }
            let (_, natural, _, _) = body.measure(gtk::Orientation::Vertical, width);
            natural > 0 && (adjustment.upper() - f64::from(natural)).abs() < 1.0
        };
        assert!(
            crate::settle_until(async || laid_out()).await,
            "the server fields were never laid out"
        );
        crate::settle();
        assert!(laid_out(), "the form was still growing");
        assert!(
            adjustment.upper() <= adjustment.page_size() + 1.0,
            "the server details need scrolling: {} of {} px show",
            adjustment.page_size(),
            adjustment.upper()
        );
        let bounds = connect
            .compute_bounds(&scroller)
            .expect("Connect is inside the scrolled body");
        assert!(
            f64::from(bounds.y() + bounds.height()) <= f64::from(scroller.height()) + 1.0,
            "Connect sits below the fold: its bottom at {}px of a {}px body",
            bounds.y() + bounds.height(),
            scroller.height()
        );
    });
}

/// The dialog covers the window and takes a click meant for it. A real click lands on the dialog
/// host's scrim, proving the window's capture gesture (`click_through_dialog`) is reached from
/// there.
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
        let picked = window
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .expect("something is under the close button");
        assert!(
            picked != close && !picked.is_ancestor(&close),
            "the form's scrim should be over the close button, or this tests nothing: \
             {} is",
            picked.type_().name()
        );
        support::click_at(&window, x, y, 1);
        assert!(
            crate::settle_until(async || !window.is_visible()).await,
            "the window's close button did nothing with the form open"
        );
    });
}

pub fn a_click_inside_the_form_is_the_forms_and_keeps_the_window_open() {
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
        let dialog = window.add_account_dialog().expect("the form");
        let body = dialog.child().expect("the form has content");
        support::click(&window, &body, 1);
        crate::settle();
        assert!(
            window.is_visible(),
            "a click inside the form closed the window"
        );
        assert!(
            window.add_account_dialog().is_some(),
            "a click inside the form closed the form"
        );
    });
}

fn arriving(subject: &str, minutes: i64) -> postio_account::backend::MockMessage {
    let date = (chrono::Utc::now() - chrono::Duration::minutes(minutes)).to_rfc2822();
    postio_account::backend::MockMessage::new(format!(
        "From: Ada Moreno <ada@example.com>\r\nTo: grace@example.test\r\n\
         Subject: {subject}\r\nDate: {date}\r\nMessage-ID: <{subject}@example.com>\r\n\r\n\
         A line about {subject}.\r\n"
    ))
}

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
        form.focus_password();
        crate::settle();
        add_through(
            &form,
            "Grace Okafor",
            "grace@example.test",
            "an app password",
        );
        start_sync(&window, &form).await;
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
        support::keys(&window, &["e"]);
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "e archived nothing: {:?}",
            support::subjects(&window)
        );
    });
}

fn page_says(window: &postio_gtk::window::FocusWindow) -> Option<Vec<String>> {
    let page = support::only(window, "focus-empty");
    page.is_mapped().then(|| support::texts(&page))
}

async fn sign_in(window: &postio_gtk::window::FocusWindow) {
    assert!(
        crate::settle_until(async || window.add_account_dialog().is_some()).await,
        "an empty store opened no add-account form"
    );
    let dialog = window.add_account_dialog().expect("the wizard");
    let form = form_in(dialog.upcast_ref()).expect("the account form in the dialog");
    add_through(
        &form,
        "Grace Okafor",
        "grace@example.test",
        "an app password",
    );
    start_sync(window, &form).await;
    assert!(
        crate::settle_until(async || window.add_account_dialog().is_none()).await,
        "the wizard stayed open after the save: {:?}",
        form.status()
    );
}

pub fn an_inbox_whose_first_pass_is_running_says_syncing_not_empty() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let backend = MockBackend::builder()
            .mailbox(
                postio_account::backend::MockMailbox::new("INBOX")
                    .message(arriving("First", 30))
                    .message(arriving("Second", 20)),
            )
            .build();
        // Every call takes a moment so the pass is in flight; not longer, since at 400 ms a loaded
        // machine ran past the deadline.
        backend.set_latency(std::time::Duration::from_millis(150));
        let (window, _backend) = opened(backend).await;
        sign_in(&window).await;

        assert!(
            crate::settle_until(async || page_says(&window).is_some()).await,
            "no page stood where the list is empty"
        );
        let said = page_says(&window).expect("the page");
        assert!(
            said.iter()
                .any(|line| line.starts_with("Syncing your inbox")),
            "the first pass is running and the page does not say so: {said:?}"
        );
        assert!(
            said.iter().all(|line| !line.contains("is empty")),
            "an inbox still filling said it was empty: {said:?}"
        );
        assert!(
            crate::settle_until(async || support::subjects(&window).len() == 2).await,
            "the rows never replaced the page: {:?}",
            support::subjects(&window)
        );
    });
}

pub fn an_inbox_a_finished_pass_found_empty_says_empty_and_when() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let backend = MockBackend::builder()
            .mailbox(postio_account::backend::MockMailbox::new("INBOX"))
            .build();
        let (window, _backend) = opened(backend).await;
        sign_in(&window).await;

        assert!(
            crate::settle_until(async || {
                page_says(&window).is_some_and(|said| {
                    said.contains(&"Inbox is empty".to_owned())
                        && said.iter().any(|line| line.starts_with("Synced "))
                })
            })
            .await,
            "the finished pass never said empty and when: {:?}",
            page_says(&window)
        );
    });
}

pub fn a_first_sync_that_fails_says_so_rather_than_empty() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let backend = MockBackend::builder()
            .mailbox(postio_account::backend::MockMailbox::new("INBOX"))
            .build();
        // Slow enough that the fault lands before the pass's first call is answered.
        backend.set_latency(std::time::Duration::from_millis(600));
        let (window, backend) = opened(backend).await;
        sign_in(&window).await;
        backend.fail_all(postio_account::backend::Fault::AuthFailed);

        assert!(
            crate::settle_until(async || {
                page_says(&window)
                    .is_some_and(|said| said.iter().any(|line| line.starts_with("Sync failed")))
            })
            .await,
            "a failing first sync did not say so: {:?}",
            page_says(&window)
        );
        let said = page_says(&window).expect("the page");
        assert!(
            said.iter()
                .all(|line| !line.contains("is empty") && !line.starts_with("Syncing")),
            "{said:?}"
        );
    });
}

pub fn an_interrupted_first_pass_still_says_syncing_not_empty() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let backend = MockBackend::builder()
            .mailbox(
                postio_account::backend::MockMailbox::new("INBOX").message(arriving("First", 30)),
            )
            .build();
        backend.set_latency(std::time::Duration::from_millis(600));
        let (window, backend) = opened(backend).await;
        sign_in(&window).await;
        backend.fail_all(postio_account::backend::Fault::Rejected("no".to_owned()));

        // The form's own check is the first call, the pass's first is the second, and the fault
        // answers it.
        assert!(
            crate::settle_until(async || backend.calls() >= 2).await,
            "the first pass never reached the server: {} calls",
            backend.calls()
        );
        for _ in 0..20 {
            crate::settle();
        }
        let said = page_says(&window).expect("the page stands where the list is empty");
        assert!(
            said.iter().all(|line| !line.contains("is empty")),
            "an interrupted first pass said the inbox was empty: {said:?}"
        );
    });
}
