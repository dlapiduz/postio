//! The desktop integration: application ID, desktop entry and icon.
//!
//! None of this needs a display — it is the metadata a session uses to launch
//! Postio and to draw it in a switcher, and it is easy to get subtly wrong in
//! a way nothing notices until a user has already installed it. The one thing
//! that is *not* checked here is whether the icon renders; that needs a
//! display and lives in `gtk_window.rs`.

use postio_gtk::{app, resources};

/// The desktop entry as it ships, next to the icon it names.
fn entry() -> glib::KeyFile {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(format!("{}.desktop", app::APP_ID));
    assert!(
        path.exists(),
        "the desktop entry must be named after the application ID: {}",
        path.display()
    );

    let key_file = glib::KeyFile::new();
    key_file
        .load_from_file(&path, glib::KeyFileFlags::NONE)
        .expect("the desktop entry should be a valid key file");
    key_file
}

fn value(key: &str) -> String {
    entry()
        .value(glib::KEY_FILE_DESKTOP_GROUP, key)
        .unwrap_or_else(|_| panic!("the desktop entry should set {key}"))
        .to_string()
}

#[test]
fn the_application_id_is_a_valid_reverse_dns_name() {
    assert_eq!(app::APP_ID, "dev.postio.Postio");
    assert!(
        gio::Application::id_is_valid(app::APP_ID),
        "GApplication would refuse `{}`",
        app::APP_ID
    );
}

#[test]
fn the_desktop_entry_describes_a_mail_client() {
    assert_eq!(value("Type"), "Application");
    assert_eq!(value("Name"), "Postio");
    assert!(!value("Comment").is_empty());

    // The binary, not the crate: `Exec` names what actually lands on PATH.
    assert!(
        value("Exec").starts_with(app::BINARY),
        "Exec should launch `{}`, got `{}`",
        app::BINARY,
        value("Exec")
    );
    assert!(
        !entry()
            .boolean(glib::KEY_FILE_DESKTOP_GROUP, "Terminal")
            .unwrap()
    );

    let categories = value("Categories");
    for wanted in ["Network", "Email"] {
        assert!(
            categories.split(';').any(|c| c == wanted),
            "Categories should contain {wanted}, got `{categories}`"
        );
    }

    // Wayland matches a window to its entry by the app ID; without this the
    // session shows a generic icon and the wrong name in the switcher.
    assert_eq!(value("StartupWMClass"), app::APP_ID);

    // A mail client that cannot be the system's mailto: handler is a toy.
    assert!(
        value("MimeType")
            .split(';')
            .any(|m| m == "x-scheme-handler/mailto"),
        "Postio should offer itself as the mailto: handler"
    );
}

#[test]
fn the_desktop_entry_passes_the_freedesktop_validator() {
    let Ok(validator) = which("desktop-file-validate") else {
        eprintln!("skipping: desktop-file-validate is not installed");
        return;
    };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(format!("{}.desktop", app::APP_ID));
    let out = std::process::Command::new(validator)
        .arg(&path)
        .output()
        .expect("desktop-file-validate should run");
    assert!(
        out.status.success(),
        "desktop-file-validate rejected the entry:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_icon_the_entry_names_ships_in_the_bundle() {
    // GTK looks an icon up by name inside an icon-theme directory layout, so
    // the resource path has to be exactly this shape or the lookup silently
    // falls back to `image-missing`.
    let icon = format!("{}/scalable/apps/{}.svg", resources::ICONS, value("Icon"));
    let bundled = resources::walk(resources::ICONS);
    assert!(
        bundled.contains(&icon),
        "the icon `{icon}` named by the desktop entry is not in the bundle: {bundled:#?}"
    );

    let bytes = resources::read(&icon).expect("the icon should be readable");
    let svg = String::from_utf8(bytes.to_vec()).expect("the icon should be UTF-8 SVG");
    assert!(svg.contains("<svg"), "the icon should be an SVG");
    assert!(
        svg.contains("viewBox"),
        "the icon needs a viewBox to scale to every size the shell asks for"
    );
}

fn which(program: &str) -> Result<std::path::PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|p| p.is_file())
        .ok_or(())
}

/// What the window tells the compositor it belongs to.
///
/// Every other assertion in this file is about the desktop entry, and an
/// entry nothing consults is worth nothing. GNOME matches a window to its
/// entry by the Wayland `app_id`, which GDK takes from `g_get_prgname()` —
/// and that defaults to the *binary* name, `postio`. So the session looked
/// for `postio.desktop`, found nothing, and drew the fallback icon under a
/// generic name, with `dev.postio.Postio.desktop` sitting correctly beside
/// it the whole time. Reported against the 0.4.2 Flatpak, where every case
/// above passed.
///
/// `build()` rather than a helper, because what regresses is the *call*
/// going missing, not the setting being wrong.
#[test]
fn the_window_says_which_application_it_is() {
    let _app = app::build();

    let reported = glib::prgname();
    assert_eq!(
        reported.as_ref().map(|name| name.as_str()),
        Some(app::APP_ID),
        "the window will tell the compositor it is {reported:?}, so a session \
         looks for that desktop entry rather than {}.desktop — which is how a \
         correct entry still produces a default icon and a generic name",
        app::APP_ID
    );
}

/// The Flatpak installs the raster sizes a session asks for.
///
/// Two separate things are called "the icon". The *gresource* bundle, which
/// `the_icon_the_entry_names_ships_in_the_bundle` above checks, is the theme
/// the application carries for its own widgets. This is the other one: what
/// the manifest installs into `/app/share/icons/hicolor` for the session to
/// draw in the switcher and the dash. Nothing checked it, and it shipped 16,
/// 32 and 128 only — so GNOME, which asks for 48 and 64, had a 128 to scale
/// and drew nothing. A blank icon in the app switcher is what that looks
/// like from outside, with `Icon=` correct, the entry exported and resolving,
/// and both icon caches listing the name.
///
/// # Why the scalable SVG is not in this list
///
/// It cannot be installed. `appstreamcli compose` runs over the finished
/// tree, loads every icon through gdk-pixbuf, and **the GNOME SDK carries
/// `librsvg` but no gdk-pixbuf SVG loader** — so an SVG under
/// `hicolor/scalable/apps` is a `file-read-error`, the compose emits no
/// component at all (`filters-but-no-output`), and the build fails. That is
/// not hypothetical: installing it was tried, it broke the build, and
/// `release.yml` runs the same step, so it would have broken the next
/// release the way the malformed metainfo did.
///
/// So the vector stays in the gresource bundle, where the application renders
/// it itself, and the session gets rasters at the sizes it actually asks for.
#[test]
fn the_flatpak_installs_the_icon_sizes_a_session_asks_for() {
    // 16 and 32 are hand-drawn rather than downscales -- `Design/icons/`
    // carries the optical-sizing rule -- and 48, 64 and 128 cover what GNOME
    // requests for the switcher, the dash and the overview.
    const REQUIRED: [&str; 5] = ["16x16", "32x32", "48x48", "64x64", "128x128"];

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the repository root");
    let manifest = std::fs::read_to_string(root.join("flatpak/dev.postio.Postio.json"))
        .expect("the Flatpak manifest should be readable");

    let mut missing = Vec::new();
    for size in REQUIRED {
        let relative = format!("crates/postio-gtk/data/icons/{size}/apps/dev.postio.Postio.png");
        if !root.join(&relative).exists() {
            missing.push(format!("{relative} (not in the repository)"));
        } else if !manifest.contains(&relative) {
            missing.push(format!("{relative} (never installed)"));
        }
    }

    assert!(
        missing.is_empty(),
        "the session will have no icon at these sizes: {missing:#?} — which \
         is drawn as nothing at all, not as a fallback"
    );
}

/// The scalable icon stays out of the bundle, and this says so on purpose.
///
/// Without an assertion the obvious "improvement" is to install it, which
/// builds fine locally right up until `appstreamcli` runs.
#[test]
fn the_flatpak_does_not_install_a_scalable_icon() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the repository root");
    let manifest = std::fs::read_to_string(root.join("flatpak/dev.postio.Postio.json"))
        .expect("the Flatpak manifest should be readable");

    assert!(
        !manifest.contains("icons/scalable"),
        "the manifest installs a scalable icon. The GNOME SDK has librsvg but \
         no gdk-pixbuf SVG loader, so `appstreamcli compose` cannot read it: \
         the component is dropped, the compose reports \
         `filters-but-no-output`, and the build fails -- in CI as well, since \
         `release.yml` runs the same step."
    );
}
