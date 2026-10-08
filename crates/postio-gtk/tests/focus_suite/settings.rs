//! Settings in Focus (T234; screens.md, "Settings"): `mod+comma` and the
//! main menu open the shared settings window in a dialog in the message
//! dialog's frame; every section Focus shows is reachable and Appearance is
//! not; Escape, `mod+comma` and the X close it; a change made there is
//! written and reaches what it changes. And `mod+e` (T235).
//!
//! Keys go through `support::deliver_with` and clicks through
//! `support::click`, the way GTK delivers them.

use adw::prelude::AdwDialogExt;
use gtk::gdk::ModifierType;
use gtk::glib;
use gtk::prelude::*;
use postio_ui::focus_dialog;

use crate::support::{self, Fixture};

/// A window over `fixture`'s store, under a `config.toml` holding `text`
/// that it follows live, with its first rows on screen. The directory holds
/// the file for as long as the case keeps it.
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
    // Followed when the machine has an inotify instance to spare: inotify is
    // shared machine-wide. A case about a reload asserts it has one
    // (`watched`); the rest do not need it.
    WATCHED.with(|watched| watched.set(session.follow_config(&window, &path)));
    support::keep(session);
    assert!(
        crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
        "the store's inbox never reached the screen"
    );
    (window, directory, path)
}

thread_local! {
    /// Whether the last [`open_under`] is following its file.
    static WATCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the window [`open_under`] last opened follows its file live.
pub fn watched() -> bool {
    WATCHED.with(std::cell::Cell::get)
}

/// One message in the inbox, and a window over it under `text`.
pub async fn one_message_under(
    text: &str,
) -> (
    Fixture,
    postio_gtk::window::FocusWindow,
    tempfile::TempDir,
    std::path::PathBuf,
) {
    let fixture = Fixture::empty().await;
    fixture
        .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
        .await;
    let (window, directory, path) = open_under(&fixture, text).await;
    (fixture, window, directory, path)
}

/// `mod+comma`, as the keyboard sends it.
fn mod_comma(window: &postio_gtk::window::FocusWindow) {
    support::deliver_with(window, "comma", ModifierType::CONTROL_MASK);
}

/// The settings dialog, once it is over the window and laid out.
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
        // Say what there was, for the failure that follows.
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

/// Whether Settings has gone from over the window.
async fn settings_gone(window: &postio_gtk::window::FocusWindow) -> bool {
    crate::settle_until(async || window.settings_dialog().is_none()).await
}

/// The section rows the settings list shows, by their names.
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

/// Scroll the pane `widget` is in until it is in view.
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

/// The pane title Settings shows now.
fn pane_title(dialog: &adw::Dialog) -> String {
    support::texts(&support::only(dialog, "postio-settings-pane-title")).join(" ")
}

pub fn mod_comma_and_the_menu_open_settings_in_focuss_frame() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _directory, _path) = one_message_under("").await;
        assert!(window.settings_dialog().is_none(), "nothing is open yet");

        // `mod+comma` opens it.
        mod_comma(&window);
        let dialog = settings_shown(&window)
            .await
            .expect("mod+comma opened Settings");

        // In the message dialog's frame: its size for this window, its
        // header, the title centred, and the one X at the right end.
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

        // `mod+comma` again closes it, back to the list as it was.
        mod_comma(&window);
        assert!(settings_gone(&window).await, "mod+comma closed Settings");
        assert_eq!(support::subjects(&window), ["Budget"]);

        // The main menu's Settings item runs the same command.
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

        // Escape closes it.
        support::deliver(&window, "Escape");
        assert!(settings_gone(&window).await, "Escape closed Settings");

        // And the X.
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
        if !support::display() {
            return;
        }
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
            "Filtering",
            "Saved searches",
            "Composing",
            "Keyboard",
            "Sync & storage",
            "Privacy",
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

/// Keyboard lists Focus's commands and no other app's.
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
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
            .await;
        // An unsigned sending identity: nothing composes without one, and an
        // unsigned one leaves the account's default the only signature.
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

        // The account's row opens its detail view.
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
        // Two signatures, so choosing the second moves the default: the
        // first is what an account with no default shows.
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

        // It is written to the store...
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

        // ...and, made the account's default in its detail view, it signs
        // the next message composed in Focus.
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

pub fn a_folder_left_out_of_backfill_is_written_and_shown() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, window, _directory, _path) = one_message_under("").await;
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        let sync = || {
            section_rows(&dialog)
                .into_iter()
                .find(|(name, _)| name == "Sync & storage")
                .map(|(_, row)| row)
        };
        assert!(
            crate::settle_until(async || sync().is_some()).await,
            "Sync & storage is listed"
        );
        let sync = sync().expect("Sync & storage's row");
        support::click(&window, &sync, 1);
        let panel = window.settings().expect("Settings is built");
        let trash = || {
            panel
                .panel()
                .backfill_check("Trash")
                .filter(|check| check.is_mapped() && check.height() > 0)
        };
        assert!(
            crate::settle_until(async || trash().is_some()).await,
            "Sync & storage lists the account's folders"
        );
        assert!(
            trash().is_some_and(|check| check.is_active()),
            "every folder backs up by default (ADR 0016)"
        );
        // Brought into view, as a person scrolls to it.
        scroll_into_view(&trash().expect("Trash's check"));
        support::settle_still(&trash().expect("Trash's check"), &dialog);
        support::click(&window, &trash().expect("Trash's check"), 1);

        let database = fixture.database.clone();
        let account = fixture.account.id;
        let excluded = || {
            let database = database.clone();
            async move {
                let connection = database.connect().await.expect("a connection");
                postio_storage::repository::MailboxRepository::new(&connection)
                    .list_for_account(account)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .any(|mailbox| mailbox.path == "Trash" && mailbox.backfill_excluded)
            }
        };
        assert!(
            crate::settle_until(excluded).await,
            "clearing Trash's check skips its backfill in the store"
        );
        assert!(
            crate::settle_until(async || panel
                .panel()
                .backfill_check("Trash")
                .is_some_and(|check| !check.is_active()))
            .await,
            "and the check, drawn again from the store's answer, stays clear"
        );
    });
}

pub fn mod_e_opens_config_toml_in_the_persons_editor() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _directory, path) = one_message_under("").await;
        let opened: std::rc::Rc<std::cell::RefCell<Vec<std::path::PathBuf>>> =
            std::rc::Rc::default();
        window.set_editor({
            let opened = std::rc::Rc::clone(&opened);
            move |path| opened.borrow_mut().push(path.to_path_buf())
        });

        // From the list.
        support::deliver_with(&window, "e", ModifierType::CONTROL_MASK);
        crate::settle();
        assert_eq!(
            opened.borrow().as_slice(),
            std::slice::from_ref(&path),
            "mod+e from the list opened config.toml"
        );

        // And from Settings, by its key and by its foot strip's button.
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        support::deliver_with(&window, "e", ModifierType::CONTROL_MASK);
        crate::settle();
        assert_eq!(opened.borrow().len(), 2, "mod+e from Settings opened it");
        let button = support::with_class(&dialog, "postio-settings-editor")
            .into_iter()
            .next()
            .expect("the foot strip's Open in $EDITOR");
        support::click(&window, &button, 1);
        assert_eq!(
            opened.borrow().as_slice(),
            [path.clone(), path.clone(), path],
            "the foot strip's button opened it"
        );
    });
}

/// Filters (classic-parity row 42): deleting a saved search in Settings
/// writes `[saved_searches]`, and Focus follows the file, so `alt+1` runs the one
/// that is first now.
pub fn a_saved_search_deleted_in_settings_leaves_alt_1_to_the_next() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _directory, path) = one_message_under(
            "[saved_searches.waiting]\nquery = \"from:juno\"\npinned = true\norder = 1\n\
             name = \"Waiting on reply\"\n\n\
             [saved_searches.atlas]\nquery = \"subject:atlas\"\npinned = true\norder = 2\n\
             name = \"Atlas\"\n",
        )
        .await;
        assert!(watched(), "the config is watched");
        mod_comma(&window);
        let dialog = settings_shown(&window).await.expect("Settings opened");
        let filters = || {
            section_rows(&dialog)
                .into_iter()
                .find(|(name, _)| name == "Saved searches")
                .map(|(_, row)| row)
        };
        assert!(crate::settle_until(async || filters().is_some()).await);
        support::click(&window, &filters().expect("Filters"), 1);
        let delete = || {
            support::with_class(&dialog, "postio-settings-filter-delete")
                .into_iter()
                .find(|button| button.is_mapped() && button.width() > 0)
        };
        assert!(
            crate::settle_until(async || delete().is_some()).await,
            "Filters lists the saved searches, each with Delete"
        );
        // The Saved searches pane has just been switched to; its rows settle
        // into place before the press, or the press and release land apart.
        let first = delete().expect("the first Delete");
        support::settle_still(&first, &dialog);
        support::click(&window, &first, 1);
        assert!(
            crate::settle_until(async || {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                !text.contains("from:juno") && text.contains("subject:atlas")
            })
            .await,
            "the first saved search left config.toml, and the second stayed: {}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );

        support::deliver(&window, "Escape");
        assert!(settings_gone(&window).await);
        assert!(
            crate::settle_until(async || {
                support::deliver_with(&window, "1", ModifierType::ALT_MASK);
                let opened = window.bar().is_some_and(|bar| bar.is_open());
                let query = window.bar().map(|bar| bar.query()).unwrap_or_default();
                if opened && query != "subject:atlas" {
                    support::deliver(&window, "Escape");
                }
                opened && query == "subject:atlas"
            })
            .await,
            "alt+1 runs Atlas now: {:?}",
            window.bar().map(|bar| bar.query())
        );
    });
}

/// Opening the signature editor puts the keyboard in its name field and
/// heads it as a new signature for the account, so the first thing typed
/// names the signature rather than searching Settings.
pub fn the_signature_editor_has_the_keyboard_and_says_what_it_is() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
            .await;
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

/// Settings' Filtering page (spec 007 US9, FR-119): it says what filtering
/// does and today's count, its switch writes `[focus] filtering` and the
/// inbox's strip follows the file, and Open Filtered goes there.
pub fn filtering_turned_off_in_settings_is_written_and_the_strip_follows() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
            .await;
        // Filtered a minute and two ago by the real clock: today.
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

        // Off: written to the file, said on the page, and the inbox's strip
        // stops counting (C10).
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

        // On again.
        support::click(&window, &switch, 1);
        assert!(
            crate::settle_until(async || {
                chrome.filtered_today_said().as_deref() == Some("2 filtered today")
            })
            .await,
            "turned on again, the strip counts again: {:?}",
            chrome.filtered_today_said()
        );

        // Open Filtered leaves Settings for Filtered.
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

/// Settings' Filtering lists can be undone where they are read: a pinned
/// sender has "Filter again", a turned-off marker "Turn back on", and each
/// writes `config.toml` without the entry, leaving the rest of it.
pub fn a_pinned_sender_and_a_turned_off_marker_are_taken_back_in_settings() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "A line.", 10)
            .await;
        let (window, _directory, path) = open_under(
            &fixture,
            "[focus]\nfiltering = true\n\n[focus.filter]\nnever = [\"@example.net\"]\n\
             stop_markers = [{ sender = \"news@ledger.example\", kind = \"question\" }]\n",
        )
        .await;
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
            "Filtering shows its page"
        );
        let said = || support::texts(&dialog);
        assert!(
            crate::settle_until(async || said().iter().any(|text| text == "Filter again")).await,
            "a pinned sender offers Filter again: {:?}",
            said()
        );
        assert!(
            said().iter().any(|text| text == "Turn back on"),
            "a turned-off marker offers Turn back on: {:?}",
            said()
        );

        // The page has just slid in; on CI the press landed mid-slide.
        let again = support::only(&dialog, "postio-settings-filtering-undo-never");
        support::settle_still(&again, &dialog);
        support::click(&window, &again, 1);
        assert!(
            crate::settle_until(async || {
                !std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("@example.net")
            })
            .await,
            "Filter again wrote the file without the pin: {}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        let back = support::only(&dialog, "postio-settings-filtering-undo-stopped");
        support::settle_still(&back, &dialog);
        support::click(&window, &back, 1);
        assert!(
            crate::settle_until(async || {
                !std::fs::read_to_string(&path)
                    .unwrap_or_default()
                    .contains("news@ledger.example")
            })
            .await,
            "Turn back on wrote the file without the marker: {}",
            std::fs::read_to_string(&path).unwrap_or_default()
        );
        assert!(
            std::fs::read_to_string(&path)
                .unwrap_or_default()
                .contains("filtering = true"),
            "and left the rest of the file"
        );
        assert!(
            crate::settle_until(async || {
                !said()
                    .iter()
                    .any(|text| text == "Filter again" || text == "Turn back on")
            })
            .await,
            "both lists are empty on the page: {:?}",
            said()
        );
    });
}
