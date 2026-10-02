//! Focus is packaged as a second launcher in the desktop Flatpak (spec 007,
//! research R3), not as a Flatpak of its own.
//!
//! None of this needs a display: it is the metadata a session uses to find
//! and launch Focus, and it goes wrong in ways nothing notices until a user
//! has installed it. Each part is checked against the others. A desktop
//! entry the manifest never installs is not shipped. A binary the entry
//! names but the manifest never builds is a launcher that does nothing. And
//! a bundle the release never looked inside can be missing either one.
//! `postio-gtk`'s `desktop_entry.rs` checks the same things for the classic
//! app, and `postio-tui`'s `packaging.rs` checks the grants the packages
//! share.

use std::path::{Path, PathBuf};

use postio_focus::app::APP_ID;

/// The binary the manifest builds and the entry launches.
const BINARY: &str = "postio-focus";

/// Where the desktop entry is installed in the sandbox.
fn installed_entry() -> String {
    format!("/app/share/applications/{APP_ID}.desktop")
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// The desktop entry as it ships, named after the application id.
fn entry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(format!("{APP_ID}.desktop"))
}

fn entry() -> glib::KeyFile {
    let path = entry_path();
    assert!(
        path.exists(),
        "the desktop entry must be named after the application id: {}",
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

fn list(key: &str) -> Vec<String> {
    value(key)
        .split(';')
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_desktop_entry_describes_focus() {
    assert_eq!(value("Type"), "Application");
    assert_eq!(value("Name"), "Postio Focus");
    assert!(!value("Comment").is_empty());
    assert!(
        value("Exec").split_whitespace().next() == Some(BINARY),
        "Exec should launch `{BINARY}`, got `{}`",
        value("Exec")
    );
    assert!(
        !entry()
            .boolean(glib::KEY_FILE_DESKTOP_GROUP, "Terminal")
            .unwrap()
    );
    let categories = list("Categories");
    for wanted in ["Network", "Email"] {
        assert!(
            categories.iter().any(|category| category == wanted),
            "Categories should contain {wanted}, got {categories:?}"
        );
    }
    // Wayland matches a window to its entry by the app id.
    assert_eq!(value("StartupWMClass"), APP_ID);
    // The one icon the package installs is the desktop app's: Focus is a
    // second launcher in the same package, with nothing of its own to draw.
    assert_eq!(value("Icon"), postio_focus::app::ICON_NAME);
}

/// `postio://` links written into notes (spec FR-185) open Focus, and only
/// Focus: the classic app keeps `mailto:`, and two launchers claiming one
/// scheme leave the choice to whichever the desktop reads first.
#[test]
fn focus_handles_postio_links_and_leaves_mailto_to_the_desktop_app() {
    let types = list("MimeType");
    assert!(
        types.iter().any(|kind| kind == "x-scheme-handler/postio"),
        "Focus should register itself for postio: links, got {types:?}"
    );
    assert!(
        !types.iter().any(|kind| kind == "x-scheme-handler/mailto"),
        "mailto: is the desktop app's, got {types:?}"
    );
    // A handler with no field code is launched without the link.
    let exec = value("Exec");
    assert!(
        exec.contains("%u") || exec.contains("%U"),
        "Exec should pass the link on, got `{exec}`"
    );
}

#[test]
fn the_entry_is_named_after_the_id_and_its_icon_ships_in_the_package() {
    assert_eq!(
        entry_path().file_name().and_then(|name| name.to_str()),
        Some(format!("{APP_ID}.desktop").as_str())
    );
    // `Icon=` is a theme name: the package installs it as an SVG, and the
    // symbolic variant, under that name.
    let icon = postio_focus::app::ICON_NAME;
    let manifest = read("flatpak/dev.postio.Postio.json");
    for installed in [
        format!("/app/share/icons/hicolor/scalable/apps/{icon}.svg"),
        format!("/app/share/icons/hicolor/symbolic/apps/{icon}-symbolic.svg"),
    ] {
        assert!(
            manifest.contains(&installed),
            "the manifest never installs {installed}"
        );
    }
    assert!(
        root()
            .join(format!(
                "crates/postio-widgets/data/icons/scalable/apps/{icon}.svg"
            ))
            .exists()
    );
}

#[test]
fn the_desktop_entry_passes_the_freedesktop_validator() {
    let Some(validator) = which("desktop-file-validate") else {
        eprintln!("skipping: desktop-file-validate is not installed");
        return;
    };
    let out = std::process::Command::new(validator)
        .arg(entry_path())
        .output()
        .expect("desktop-file-validate should run");
    assert!(
        out.status.success(),
        "desktop-file-validate rejected the entry:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The desktop Flatpak builds Focus, installs it, and installs its entry.
#[test]
fn the_desktop_flatpak_builds_and_installs_focus() {
    let manifest = read("flatpak/dev.postio.Postio.json");
    let commands: Vec<&str> = manifest
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('"'))
        .collect();

    let built = format!("--package {BINARY} --bin {BINARY}");
    assert!(
        commands
            .iter()
            .any(|line| line.contains("cargo") && line.contains(&built)),
        "the manifest never builds {BINARY}"
    );
    let binary = format!("target/release/{BINARY} /app/bin/{BINARY}\"");
    assert!(
        commands.iter().any(|line| line.contains(&binary)),
        "the manifest never installs /app/bin/{BINARY}"
    );
    let source = format!("crates/postio-focus/data/{APP_ID}.desktop");
    let line = commands
        .iter()
        .find(|line| line.contains(&source))
        .unwrap_or_else(|| panic!("the manifest never installs {source}"));
    assert!(
        line.contains(&installed_entry()),
        "{source} should be installed as {}, but the manifest says:\n{line}",
        installed_entry()
    );
}

/// One package, one AppStream component: GNOME Software shows the desktop
/// app's page, and that page names both launchers. A second component of
/// Focus's own would describe no package, since a Flatpak's catalog entry
/// is composed for its own app id only.
#[test]
fn the_desktop_apps_metainfo_names_focus_as_a_second_launcher() {
    let metainfo = read("crates/postio-focus/data/dev.postio.Postio.metainfo.xml");
    let launchable = format!("<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>");
    assert!(
        metainfo.contains(&launchable),
        "the metainfo should list {APP_ID}.desktop as a launchable"
    );
    let provided = format!("<binary>{BINARY}</binary>");
    assert!(
        metainfo.contains(&provided),
        "the metainfo should say the package provides {BINARY}"
    );
    assert!(
        !Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join(format!("{APP_ID}.metainfo.xml"))
            .exists(),
        "Focus is not a package of its own, so it has no component of its own"
    );
}

/// The release looks inside the bundle it is about to publish for both
/// apps, so a manifest that stops building one fails the release rather
/// than shipping without it.
#[test]
fn the_release_checks_the_bundle_carries_both_apps() {
    let workflow = read(".github/workflows/release.yml");
    let job = workflow
        .split("\n  flatpak:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n  tui-flatpak:\n").next())
        .expect("release.yml should have a `flatpak` job before `tui-flatpak`");
    for installed in [
        "files/bin/postio".to_owned(),
        format!("files/bin/{BINARY}"),
        "files/share/applications/dev.postio.Postio.desktop".to_owned(),
        format!("files/share/applications/{APP_ID}.desktop"),
    ] {
        // Whole words on lines that run, so `bin/postio` is not satisfied by
        // `bin/postio-focus`, nor either by a comment naming it.
        assert!(
            job.lines()
                .filter(|line| !line.trim_start().starts_with('#'))
                .any(|line| line.split_whitespace().any(|word| word == installed)),
            "the `flatpak` job never checks the build for {installed}"
        );
    }
}

/// Nothing the package installs, and nothing Focus compiles in, is read
/// from the classic app's crate, which goes whole when the classic app is
/// removed: the icons and the token build are `postio-widgets`', the
/// desktop entry and metainfo are here.
#[test]
fn nothing_focus_ships_is_read_from_the_classic_crate() {
    for (what, text) in [
        (
            "the Flatpak manifest",
            read("flatpak/dev.postio.Postio.json"),
        ),
        (
            "postio-widgets' build step",
            read("crates/postio-widgets/build.rs"),
        ),
        (
            "postio-widgets' resource bundle",
            read("crates/postio-widgets/data/widgets.gresource.xml"),
        ),
    ] {
        assert!(
            !text.contains("postio-gtk/"),
            "{what} reads a file under crates/postio-gtk"
        );
    }
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}
