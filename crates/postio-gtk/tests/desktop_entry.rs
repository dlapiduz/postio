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
