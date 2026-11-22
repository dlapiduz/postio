use adw::prelude::AdwDialogExt;
use gtk::gdk::ModifierType;
use gtk::glib;
use gtk::prelude::*;
use postio_ui::focus_dialog;

use crate::support::{self, Fixture};

pub async fn open_under(
    fixture: &Fixture,
    text: &str,
) -> (
    postio_gtk::window::FocusWindow,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let directory = tempfile::tempdir().expect("a config directory");
    let path = directory.path().join("config.toml");
    std::fs::write(&path, text).expect("the config");
    let config = postio_config::Config::from_toml_str(text).expect("a config");
    let window = postio_gtk::window::FocusWindow::new(None);
    window.present();
    let session = postio_gtk::startup::adopt_at(&window, fixture.host(), &config, Some(&path));
    // Followed when the machine has an inotify instance to spare (it is shared machine-wide); a
    // reload case asserts it has one (`watched`).
    WATCHED.with(|watched| watched.set(session.follow_config(&window, &path)));
    support::keep(session);
    assert!(
        crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
        "the store's inbox never reached the screen"
    );
    (window, directory, path)
}

thread_local! {
    static WATCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn watched() -> bool {
    WATCHED.with(std::cell::Cell::get)
}

pub async fn one_message_under(
    text: &str,
) -> (
    Fixture,
    postio_gtk::window::FocusWindow,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let fixture = Fixture::empty().await;
    fixture.file_from_ada("Budget", "A line.", 10).await;
    let (window, directory, path) = open_under(&fixture, text).await;
    (fixture, window, directory, path)
}

fn mod_comma(window: &postio_gtk::window::FocusWindow) {
    support::deliver_with(window, "comma", ModifierType::CONTROL_MASK);
}

pub async fn settings_shown(window: &postio_gtk::window::FocusWindow) -> Option<adw::Dialog> {
    crate::settle_until(async || {
        window.settings().is_some_and(|settings| {
            settings.is_open() && settings.panel().is_mapped() && settings.panel().width() > 0
        })
    })
    .await
    .then(|| window.settings_dialog())
    .flatten()
    .or_else(|| {
        eprintln!(
            "Settings did not show: built {}, open {}, panel mapped {}, dialog mapped {}",
            window.settings().is_some(),
            window.settings().is_some_and(|settings| settings.is_open()),
            window
                .settings()
                .is_some_and(|settings| settings.panel().is_mapped()),
            window
                .settings_dialog()
                .is_some_and(|dialog| dialog.is_mapped()),
        );
        None
    })
}

async fn settings_gone(window: &postio_gtk::window::FocusWindow) -> bool {
    crate::settle_until(async || window.settings_dialog().is_none()).await
}

pub fn section_rows(dialog: &adw::Dialog) -> Vec<(String, gtk::Widget)> {
    support::with_class(dialog, "postio-settings-nav-row")
        .into_iter()
        .filter(|row| row.is_child_visible() && row.is_visible())
        .map(|row| {
            let name = support::texts(&row).join(" ");
            (name, row)
        })
        .collect()
}

fn scroll_into_view(widget: &impl IsA<gtk::Widget>) {
    let widget = widget.as_ref();
    let Some(scroller) = widget
        .ancestor(gtk::ScrolledWindow::static_type())
        .and_downcast::<gtk::ScrolledWindow>()
    else {
        return;
    };
    let Some(child) = scroller.child() else {
        return;
    };
    if let Some(at) = widget.compute_point(&child, &gtk::graphene::Point::new(0.0, 0.0)) {
        scroller
            .vadjustment()
            .set_value(f64::from(at.y()) - f64::from(scroller.height()) / 2.0);
    }
    crate::settle();
}

fn pane_title(dialog: &adw::Dialog) -> String {
    support::texts(&support::only(dialog, "postio-settings-pane-title")).join(" ")
}

pub fn mod_comma_and_the_menu_open_settings_in_focuss_frame() {
    crate::gtk_case(async {
        let (_fixture, window, _directory, _path) = one_message_under("").await;
        assert!(window.settings_dialog().is_none(), "nothing is open yet");

        mod_comma(&window);
        let dialog = settings_shown(&window)
            .await
            .expect("mod+comma opened Settings");

        assert!(
            crate::settle_until(async || dialog.content_width()
                == focus_dialog::dialog_width(window.width())
                && dialog.content_height() == focus_dialog::dialog_height(window.height()))
            .await,
            "Settings is the message dialog's size for this {}x{} window: {}x{}",
            window.width(),
            window.height(),
            dialog.content_width(),
            dialog.content_height()
        );
        let header = support::only(&dialog, "focus-open-header");
        assert!(
            support::texts(&header).contains(&"Settings".to_owned()),
            "the header names it: {:?}",
            support::texts(&header)
        );
        let closes = support::with_class(&header, "postio-close-button");
        assert_eq!(closes.len(), 1, "one X, in the header");
        assert!(
            support::with_class(&header, "focus-settings-search")
                .first()
                .is_some_and(|field| field.is::<gtk::SearchEntry>()),
            "the find-a-setting field is in the header"
        );

        mod_comma(&window);
        assert!(settings_gone(&window).await, "mod+comma closed Settings");
        assert_eq!(support::subjects(&window), ["Budget"]);

        let menu = support::only(&window, "focus-menu")
            .downcast::<gtk::MenuButton>()
            .expect("the main menu is a menu button");
        let model = menu.menu_model().expect("the menu has items");
        let item = (0..model.n_items())
            .find(|item| {
                model
                    .item_attribute_value(*item, "label", Some(glib::VariantTy::STRING))
                    .and_then(|label| label.get::<String>())
                    .as_deref()
                    == Some("Settings")
            })
            .expect("the menu has Settings");
        let action = model
            .item_attribute_value(item, "action", Some(glib::VariantTy::STRING))
            .and_then(|action| action.get::<String>())
            .expect("Settings runs an action");
        let target = model.item_attribute_value(item, "target", None);
        menu.activate_action(&action, target.as_ref())
            .expect("the menu item's action exists");
        assert!(
            settings_shown(&window).await.is_some(),
            "the main menu's Settings opened Settings"
        );

        support::deliver(&window, "Escape");
        assert!(settings_gone(&window).await, "Escape closed Settings");

        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("open again");
        let close = support::with_class(&dialog, "postio-close-button")
            .into_iter()
            .next()
            .expect("the X");
        support::click(&window, &close, 1);
        assert!(settings_gone(&window).await, "the X closed Settings");
    });
}

pub fn every_section_focus_shows_is_reachable_and_appearance_is_not() {
    crate::gtk_case(async {
        let (_fixture, window, _directory, _path) = one_message_under("").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        let names = || -> Vec<String> {
            section_rows(&dialog)
                .into_iter()
                .map(|(name, _)| name)
                .collect()
        };
        let shown = [
            "Accounts",
            "Contacts",
            "Filtering",
            "Saved searches",
            "Composing",
            "Keyboard",
            "Sync & storage",
            "Privacy",
            "Agents",
            "Config file",
        ];
        assert!(
            crate::settle_until(async || names() == shown).await,
            "Focus shows every section but Appearance, whose keys it does not honour: {:?}",
            names()
        );
        for (name, row) in section_rows(&dialog) {
            support::click(&window, &row, 1);
            assert!(
                crate::settle_until(async || pane_title(&dialog) == name).await,
                "clicking {name} shows its pane, not {:?}",
                pane_title(&dialog)
            );
            if name == "Keyboard" {
                keyboard_lists_focuss_commands(&dialog).await;
            }
        }
    });
}

async fn keyboard_lists_focuss_commands(dialog: &adw::Dialog) {
    let listed = || support::texts(&support::only(dialog, "postio-settings-keys-list"));
    assert!(
        crate::settle_until(async || !listed().is_empty()).await,
        "Keyboard lists commands"
    );
    let listed = listed();
    {
        assert!(
            listed
                .iter()
                .any(|text| text == "Show only what has an action"),
            "Keyboard lists a Focus command: {listed:?}"
        );
        assert!(
            !listed.iter().any(|text| text == "Toggle sidebar"),
            "and not the classic app's sidebar: {listed:?}"
        );
    }
}

pub fn a_signature_made_in_settings_signs_the_next_message() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "A line.", 10).await;
        // An unsigned sending identity: nothing composes without one, and an unsigned one leaves
        // the account's default the only signature.
        {
            let connection = fixture.database.connect().await.expect("a connection");
            let mut identity =
                postio_model::Identity::new(fixture.account.id, fixture.account.address.clone());
            identity.is_default = true;
            postio_storage::repository::IdentityRepository::new(&connection)
                .create(&mut identity)
                .await
                .expect("a sending identity");
        }
        let (window, _directory, _path) = open_under(&fixture, "").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");

        let row = || {
            support::with_class(&dialog, "postio-settings-account-row")
                .into_iter()
                .find(|row| row.is_mapped())
        };
        assert!(
            crate::settle_until(async || row().is_some()).await,
            "the account has a row"
        );
        let row = row().expect("the account's row");
        support::click(&window, &row, 1);
        for (called, body) in [("Plain", "Ada"), ("Work", "Ada Moreno\nAtlas team")] {
            let add = support::button_labelled(&dialog, "Add signature");
            scroll_into_view(&add);
            support::settle_still(&add, &dialog);
            support::click(&window, &add, 1);
            assert!(
                crate::settle_until(async || {
                    support::with_class(&dialog, "postio-settings-signature-editor")
                        .iter()
                        .any(|editor| editor.is_mapped())
                })
                .await,
                "Add signature opened the editor for {called}"
            );
            let name = support::only(&dialog, "postio-settings-signature-name")
                .downcast::<gtk::Entry>()
                .expect("the name is an entry");
            name.set_text(called);
            let text = support::only(&dialog, "postio-settings-signature-text")
                .downcast::<gtk::TextView>()
                .expect("the body is a text view");
            text.buffer().set_text(body);
            let save = support::button_labelled(&dialog, "Save");
            support::click(&window, &save, 1);
            assert!(
                crate::settle_until(async || {
                    support::with_class(&dialog, "postio-settings-signature-row")
                        .iter()
                        .any(|row| support::texts(row) == [called])
                })
                .await,
                "{called} is listed on the account once saved"
            );
        }

        let account = fixture.account.id;
        let database = fixture.database.clone();
        let stored = || {
            let database = database.clone();
            async move {
                let connection = database.connect().await.expect("a connection");
                postio_storage::repository::SignatureRepository::new(&connection)
                    .list_for_account(account)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .any(|signature| signature.name == "Work")
            }
        };
        assert!(
            crate::settle_until(stored).await,
            "the signature reached the store"
        );

        let picker = || {
            support::with_class(&dialog, "postio-settings-account-detail-signature")
                .into_iter()
                .find_map(|widget| widget.downcast::<gtk::DropDown>().ok())
        };
        let offers_work = || {
            picker()
                .and_then(|picker| picker.model())
                .and_then(|names| {
                    (0..names.n_items()).find(|at| {
                        names
                            .item(*at)
                            .and_downcast::<gtk::StringObject>()
                            .is_some_and(|name| name.string() == "Work")
                    })
                })
        };
        assert!(
            crate::settle_until(async || offers_work().is_some()).await,
            "the new signature is offered as the account's default"
        );
        let work = offers_work().expect("Work is offered");
        let picker = picker().expect("the default signature's drop-down");
        picker.set_selected(work);
        let is_default = || {
            let database = database.clone();
            async move {
                let connection = database.connect().await.expect("a connection");
                postio_storage::repository::AccountRepository::new(&connection)
                    .get(account)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|account| account.default_signature_id.is_some())
            }
        };
        assert!(
            crate::settle_until(is_default).await,
            "choosing it made it the account's default"
        );
        support::deliver(&window, "Escape");
        assert!(settings_gone(&window).await);
        support::deliver(&window, "c");
        assert!(
            crate::settle_until(async || window.composer().is_some_and(|composer| composer
                .draft()
                .body
                .text
                .unwrap_or_default()
                .contains("Atlas team")))
            .await,
            "the next message is signed with it: {:?}",
            window.composer().map(|composer| composer.draft().body.text)
        );
    });
}

pub fn the_signature_editor_has_the_keyboard_and_says_what_it_is() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "A line.", 10).await;
        let (window, _directory, _path) = open_under(&fixture, "").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        let row = || {
            support::with_class(&dialog, "postio-settings-account-row")
                .into_iter()
                .find(|row| row.is_mapped())
        };
        assert!(
            crate::settle_until(async || row().is_some()).await,
            "the account has a row"
        );
        support::click(&window, &row().expect("the account's row"), 1);
        let add = support::button_labelled(&dialog, "Add signature");
        scroll_into_view(&add);
        support::settle_still(&add, &dialog);
        support::click(&window, &add, 1);
        assert!(
            crate::settle_until(async || {
                support::with_class(&dialog, "postio-settings-signature-editor")
                    .iter()
                    .any(|editor| editor.is_mapped())
            })
            .await,
            "Add signature opened no editor"
        );
        let name = support::only(&dialog, "postio-settings-signature-name");
        let held = || {
            dialog
                .focus()
                .is_some_and(|focus| focus == name || focus.is_ancestor(&name))
        };
        assert!(
            crate::settle_until(async || held()).await,
            "the keyboard is on {:?}, not in the signature's name",
            dialog.focus().map(|focus| focus.type_().name())
        );
        assert!(
            support::texts(&dialog)
                .iter()
                .any(|text| text.starts_with("New signature")),
            "the editor says nothing of what it makes: {:?}",
            support::texts(&dialog)
        );
    });
}

pub fn filtering_turned_off_in_settings_is_written_and_the_strip_follows() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        fixture.file_from_ada("Budget", "A line.", 10).await;
        let ago = |minutes: i64| (support::now() - chrono::Utc::now()).num_minutes() + minutes;
        for (subject, minutes) in [("Review requested", 1), ("Build passed", 2)] {
            fixture
                .filtered(
                    ("Forge", "noreply@forge.test"),
                    subject,
                    "notification",
                    Some("Forge"),
                    ago(minutes),
                )
                .await;
        }
        let (window, _directory, path) = open_under(
            &fixture,
            "[focus]\nfiltering = true\n\n[focus.filter]\nnever = [\"@example.net\"]\n",
        )
        .await;
        assert!(watched(), "the config is watched");
        let chrome = window.chrome().expect("the strip");
        assert!(
            crate::settle_until(async || {
                chrome.filtered_today_said().as_deref() == Some("2 filtered today")
            })
            .await,
            "the strip counts today's: {:?}",
            chrome.filtered_today_said()
        );

        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        let row = section_rows(&dialog)
            .into_iter()
            .find(|(name, _)| name == "Filtering")
            .map(|(_, row)| row)
            .expect("Settings lists Filtering");
        support::click(&window, &row, 1);
        assert!(
            crate::settle_until(async || pane_title(&dialog) == "Filtering").await,
            "Filtering shows its page, not {:?}",
            pane_title(&dialog)
        );
        let said = || support::texts(&dialog);
        assert!(
            crate::settle_until(async || said().iter().any(|text| text == "2 filtered today"))
                .await,
            "the page counts what the strip counts: {:?}",
            said()
        );
        let shown = said();
        for line in [
            postio_ui::filtering::SWITCH,
            postio_ui::filtering::KEPT,
            "everyone at example.net",
            "g f",
        ] {
            assert!(
                shown.iter().any(|text| text == line),
                "the page says {line:?}: {shown:?}"
            );
        }
        assert!(
            shown
                .iter()
                .any(|text| text.contains("never reach the inbox")),
            "and what filtering does: {shown:?}"
        );

        let switch = support::only(&dialog, "postio-settings-filtering-switch");
        support::settle_still(&switch, &dialog);
        support::click(&window, &switch, 1);
        assert!(
            crate::settle_until(async || {
                std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("filtering = false")
            })
            .await,
            "the switch wrote [focus] filtering: {}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .contains("never = [\"@example.net\"]"),
            "and left the rest of [focus] where it was"
        );
        assert!(
            crate::settle_until(async || said().iter().any(|text| text.starts_with("Off:"))).await,
            "the page says filtering is off: {:?}",
            said()
        );
        assert!(
            !said().iter().any(|text| text == "2 filtered today"),
            "and counts nothing while it is: {:?}",
            said()
        );
        assert!(
            crate::settle_until(async || chrome.filtered_today_said().is_none()).await,
            "the inbox's strip followed the file: {:?}",
            chrome.filtered_today_said()
        );

        support::click(&window, &switch, 1);
        assert!(
            crate::settle_until(async || {
                chrome.filtered_today_said().as_deref() == Some("2 filtered today")
            })
            .await,
            "turned on again, the strip counts again: {:?}",
            chrome.filtered_today_said()
        );

        let open = support::only(&dialog, "postio-settings-filtering-open");
        support::click(&window, &open, 1);
        assert!(
            settings_gone(&window).await,
            "Open Filtered closed Settings"
        );
        assert!(
            crate::settle_until(async || window.filtered().is_some()).await,
            "and opened Filtered"
        );
    });
}

/// libadwaita opens a dialog by scaling it from 0.8 to 1, so until it has opened each row is
/// smaller and elsewhere every frame,
/// and a press and a release a frame later land at two places: the press waits for the row to be
/// drawn still (#1817).
pub fn a_click_into_settings_as_it_opens_waits_for_it_to_open() {
    crate::gtk_case(async {
        let (_fixture, window, _directory, _path) = one_message_under("").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        let row = section_rows(&dialog)
            .into_iter()
            .find(|(name, _)| name == "Filtering")
            .map(|(_, row)| row)
            .expect("Settings lists Filtering");

        type Sizes = Vec<(f32, f32, f32, f32)>;
        let pressed: std::rc::Rc<std::cell::RefCell<Sizes>> = std::rc::Rc::default();
        let watch = gtk::GestureClick::new();
        watch.set_propagation_phase(gtk::PropagationPhase::Capture);
        watch.connect_pressed({
            let pressed = std::rc::Rc::clone(&pressed);
            let row = row.downgrade();
            move |_, _, _, _| {
                let Some(row) = row.upgrade() else {
                    return;
                };
                let Some(native) = row.native() else {
                    return;
                };
                if let (Some(drawn), Some(own)) =
                    (row.compute_bounds(&native), row.compute_bounds(&row))
                {
                    pressed.borrow_mut().push((
                        drawn.width(),
                        drawn.height(),
                        own.width(),
                        own.height(),
                    ));
                }
            }
        });
        row.add_controller(watch);

        support::click(&window, &row, 1);
        let presses = pressed.borrow().clone();
        assert_eq!(presses.len(), 1, "one press reached Filtering's row");
        let (width, height, own_width, own_height) = presses[0];
        assert!(
            (width - own_width).abs() < 0.5 && (height - own_height).abs() < 0.5,
            "the press waited for Settings to open: the row was drawn {width}x{height}, \
             its own size is {own_width}x{own_height}"
        );
        assert!(
            crate::settle_until(async || pane_title(&dialog) == "Filtering").await,
            "clicking Filtering shows its pane, not {:?}",
            pane_title(&dialog)
        );
    });
}

pub fn agents_in_settings_shows_the_log_and_revokes_from_the_file() {
    crate::gtk_case(async {
        let fixture = Fixture::empty().await;
        let message = fixture
            .file_as(fixture.account.id, fixture.inbox, "Budget", 10)
            .await;
        let address = fixture.account.address.address.clone();
        let text = format!("[mcp.accounts.\"{address}\"]\nmailboxes = [\"INBOX\"]\n");

        let directory = tempfile::tempdir().expect("a directory");
        let path = directory.path().join("config.toml");
        std::fs::write(&path, &text).expect("the config");
        let store = directory.path().join("postio.db");
        let at = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        std::fs::write(
            directory.path().join("mcp-access.log"),
            format!(
                "{{\"at\":\"{at}\",\"tool\":\"read_message\",\"accounts\":[{}],\
                 \"mailboxes\":[{}],\"messages\":[{}],\"drafts\":[],\"labels\":[],\
                 \"outcome\":\"answered\"}}\n",
                fixture.account.id.get(),
                fixture.inbox.get(),
                message.get()
            ),
        )
        .expect("the log");
        let config = postio_config::Config::from_toml_str(&text).expect("a config");
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt_from(
            &window,
            fixture.host(),
            &config,
            Some(&path),
            Some(&store),
        ));
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the store's inbox never reached the screen"
        );

        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        let (_, agents) = section_rows(&dialog)
            .into_iter()
            .find(|(name, _)| name == "Agents")
            .expect("Agents is in the nav");
        support::click(&window, &agents, 1);

        let calls = || {
            support::with_class(&dialog, "postio-settings-agents-call")
                .iter()
                .flat_map(support::texts)
                .collect::<Vec<_>>()
        };
        assert!(
            crate::settle_until(async || calls()
                .iter()
                .any(|line| line.contains("Budget") && line.contains(&address)))
            .await,
            "the call names the message and the account, not their ids: {:?}",
            calls()
        );

        let revoke = support::with_class(&dialog, "postio-settings-agents-revoke")
            .into_iter()
            .next()
            .expect("the shared account's Revoke");
        support::click(&window, &revoke, 1);
        assert!(
            crate::settle_until(async || std::fs::read_to_string(&path)
                .is_ok_and(|written| !written.contains("[mcp")))
            .await,
            "the revoke reached the file: {:?}",
            std::fs::read_to_string(&path)
        );
    });
}
