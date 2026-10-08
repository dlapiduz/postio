//! The package is Focus (spec 007, decision C27; ADR 0043): one app named
//! Postio, the binary `postio`, the application id `dev.postio.Postio`, one
//! desktop entry, one AppStream component, in the desktop Flatpak.
//!
//! None of this needs a display: it is the metadata a session uses to find
//! and launch Postio, and it goes wrong in ways nothing notices until a user
//! has installed it. Each part is checked against the others. A desktop
//! entry the manifest never installs is not shipped. A binary the entry
//! names but the manifest never builds is a launcher that does nothing. And
//! a bundle the release never looked inside can be missing either one.
//! `postio-tui`'s `packaging.rs` checks the grants the packages share.

use std::path::{Path, PathBuf};

use postio_gtk::app::{APP_ID, ICON_NAME};

/// The binary the manifest builds and the entry launches.
const BINARY: &str = "postio";

/// The Flatpak the desktop app ships in, named after the id.
const MANIFEST: &str = "flatpak/dev.postio.Postio.json";

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

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("data")
}

/// The desktop entry as it ships, named after the application id.
fn entry_path() -> PathBuf {
    data().join(format!("{APP_ID}.desktop"))
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

/// The names of the `[[bin]]` targets a crate's manifest declares.
fn binaries(crate_dir: &str) -> Vec<String> {
    let manifest = read(&format!("{crate_dir}/Cargo.toml"));
    manifest
        .split("[[bin]]")
        .skip(1)
        .filter_map(|section| {
            section.lines().find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "name").then(|| value.trim().trim_matches('"').to_owned())
            })
        })
        .collect()
}

/// The manifest's build commands, one per JSON string line.
fn commands(manifest: &str) -> Vec<&str> {
    manifest
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('"'))
        .collect()
}

/// The app is Postio: the id the desktop app had, and the binary on PATH.
#[test]
fn focus_is_postio() {
    assert_eq!(APP_ID, "dev.postio.Postio");
    assert!(
        gio::Application::id_is_valid(APP_ID),
        "GApplication would refuse `{APP_ID}`"
    );
    assert_eq!(binaries("crates/postio-gtk"), vec![BINARY.to_owned()]);
}

#[test]
fn the_desktop_entry_describes_postio() {
    assert_eq!(value("Type"), "Application");
    assert_eq!(value("Name"), "Postio");
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
    assert_eq!(value("Icon"), ICON_NAME);
}

/// Postio is the system's `mailto:` handler, and opens the `postio://`
/// links written into notes (spec FR-185). With one launcher, nothing else
/// in the package claims either scheme.
#[test]
fn postio_handles_mailto_and_postio_links() {
    let types = list("MimeType");
    for wanted in ["x-scheme-handler/mailto", "x-scheme-handler/postio"] {
        assert!(
            types.iter().any(|kind| kind == wanted),
            "the entry should register {wanted}, got {types:?}"
        );
    }
    // A handler with no field code is launched without the link.
    let exec = value("Exec");
    assert!(
        exec.contains("%u") || exec.contains("%U"),
        "Exec should pass the link on, got `{exec}`"
    );
}

/// One app, one launcher: a second entry would be a second icon in the app
/// grid, and two entries claiming one scheme leave the choice to whichever
/// the desktop reads first.
#[test]
fn the_package_has_one_desktop_entry() {
    let entries: Vec<String> = std::fs::read_dir(data())
        .expect("the data directory")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".desktop"))
        .collect();
    assert_eq!(entries, vec![format!("{APP_ID}.desktop")]);
}

#[test]
fn the_entry_is_named_after_the_id_and_its_icon_ships_in_the_package() {
    assert_eq!(
        entry_path().file_name().and_then(|name| name.to_str()),
        Some(format!("{APP_ID}.desktop").as_str())
    );
    // `Icon=` is a theme name: the package installs it as an SVG, and the
    // symbolic variant, under that name.
    let manifest = read(MANIFEST);
    for (source, installed) in [
        (
            format!("crates/postio-widgets/data/icons/scalable/apps/{ICON_NAME}.svg"),
            format!("/app/share/icons/hicolor/scalable/apps/{ICON_NAME}.svg"),
        ),
        (
            format!("crates/postio-widgets/data/icons/scalable/apps/{ICON_NAME}-symbolic.svg"),
            format!("/app/share/icons/hicolor/symbolic/apps/{ICON_NAME}-symbolic.svg"),
        ),
    ] {
        assert!(
            root().join(&source).exists(),
            "{source} is not in the repository"
        );
        let line = manifest
            .lines()
            .find(|line| line.contains(&source))
            .unwrap_or_else(|| panic!("the manifest never installs {source}"));
        assert!(
            line.contains(&installed),
            "{source} should be installed as {installed}, but the manifest says:\n{line}"
        );
    }
}

/// The Flatpak installs the raster sizes a session asks for. 16 and 32 are
/// hand-drawn rather than downscales (`Design/icons/` carries the
/// optical-sizing rule), and 48, 64 and 128 are what GNOME asks for in the
/// dash, the overview and the switcher at 1x. A size with no file is drawn
/// as nothing at all, not as a fallback.
#[test]
fn the_flatpak_installs_the_icon_sizes_a_session_asks_for() {
    let manifest = read(MANIFEST);
    let mut missing = Vec::new();
    for size in ["16x16", "32x32", "48x48", "64x64", "128x128"] {
        let relative = format!("crates/postio-widgets/data/icons/{size}/apps/{ICON_NAME}.png");
        if !root().join(&relative).exists() {
            missing.push(format!("{relative} (not in the repository)"));
        } else if !manifest.contains(&relative) {
            missing.push(format!("{relative} (never installed)"));
        }
    }
    assert!(
        missing.is_empty(),
        "the session will have no icon at these sizes: {missing:#?}"
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

/// The desktop Flatpak builds Focus as `postio`, runs it, and installs its
/// entry and metainfo. It builds nothing else.
#[test]
fn the_desktop_flatpak_builds_and_installs_postio_only() {
    let manifest = read(MANIFEST);
    assert!(
        manifest.contains(&format!("\"command\": \"{BINARY}\"")),
        "the manifest's command should be {BINARY}"
    );
    let commands = commands(&manifest);

    let builds: Vec<&&str> = commands
        .iter()
        .filter(|line| line.contains("cargo") && line.contains(" build "))
        .collect();
    let built = format!("--package postio-gtk --bin {BINARY}");
    assert!(
        builds.len() == 1 && builds[0].contains(&built),
        "the manifest should build `{built}` and nothing else, got {builds:#?}"
    );
    let binary = format!("target/release/{BINARY} /app/bin/{BINARY}\"");
    assert!(
        commands.iter().any(|line| line.contains(&binary)),
        "the manifest never installs /app/bin/{BINARY}"
    );
    let installed_binaries: Vec<&&str> = commands
        .iter()
        .filter(|line| line.contains("/app/bin/"))
        .collect();
    assert_eq!(
        installed_binaries.len(),
        1,
        "the manifest installs more than one binary: {installed_binaries:#?}"
    );

    for (source, installed) in [
        (
            format!("crates/postio-gtk/data/{APP_ID}.desktop"),
            installed_entry(),
        ),
        (
            format!("crates/postio-gtk/data/{APP_ID}.metainfo.xml"),
            format!("/app/share/metainfo/{APP_ID}.metainfo.xml"),
        ),
    ] {
        let line = commands
            .iter()
            .find(|line| line.contains(&source))
            .unwrap_or_else(|| panic!("the manifest never installs {source}"));
        assert!(
            line.contains(&installed),
            "{source} should be installed as {installed}, but the manifest says:\n{line}"
        );
    }
    let entries: Vec<&&str> = commands
        .iter()
        .filter(|line| line.contains("/app/share/applications/"))
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "the manifest installs more than one desktop entry: {entries:#?}"
    );
}

/// One package, one app, one AppStream component: GNOME Software shows
/// Postio's page, and it names the one launcher and the one binary.
#[test]
fn the_metainfo_describes_postio_with_one_launcher() {
    let metainfo = read(&format!("crates/postio-gtk/data/{APP_ID}.metainfo.xml"));
    assert!(metainfo.contains(&format!("<id>{APP_ID}</id>")));
    assert!(metainfo.contains("<name>Postio</name>"));
    let launchables: Vec<&str> = metainfo
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<launchable"))
        .collect();
    assert_eq!(
        launchables,
        vec![format!("<launchable type=\"desktop-id\">{APP_ID}.desktop</launchable>").as_str()]
    );
    let provided: Vec<&str> = metainfo
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("<binary>"))
        .collect();
    assert_eq!(
        provided,
        vec![format!("<binary>{BINARY}</binary>").as_str()]
    );
    // The app is Postio now; "Postio Focus" was the second launcher's name.
    assert!(
        !metainfo.contains("Postio Focus"),
        "the metainfo still describes Focus as a second app"
    );
}

/// The store page shows what the app looks like: screenshots, each an image
/// the site serves (so the URL is the site's own) that exists in the
/// repository, and none of them an earlier app's.
#[test]
fn the_metainfo_shows_focus_screenshots_the_site_serves() {
    let metainfo = read(&format!("crates/postio-gtk/data/{APP_ID}.metainfo.xml"));
    let prefix = "https://dlapiduz.github.io/postio/assets/img/";
    let images: Vec<&str> = metainfo
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("<image>")?.strip_suffix("</image>"))
        .collect();
    assert!(
        images.len() >= 3,
        "the store page has too few screenshots: {images:?}"
    );
    for image in images {
        let name = image
            .strip_prefix(prefix)
            .unwrap_or_else(|| panic!("{image} is not an image the site serves"));
        assert!(
            name.starts_with("focus-"),
            "{name} is not a render of this app"
        );
        assert!(
            root().join("site/assets/img").join(name).exists(),
            "the site has no {name}"
        );
    }
}

/// The release looks inside the bundle it is about to publish, so a
/// manifest that stops building the app fails the release rather than
/// shipping without it.
#[test]
fn the_release_checks_the_bundle_carries_postio() {
    let workflow = read(".github/workflows/release.yml");
    // The `flatpak` job, up to the next job: a line indented two spaces.
    let running: Vec<&str> = workflow
        .lines()
        .skip_while(|line| *line != "  flatpak:")
        .skip(1)
        .take_while(|line| !(line.starts_with("  ") && !line.starts_with("   ")))
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect();
    assert!(
        !running.is_empty(),
        "release.yml should have a `flatpak` job"
    );
    for installed in [
        format!("files/bin/{BINARY}"),
        format!("files/share/applications/{APP_ID}.desktop"),
        format!("files/share/metainfo/{APP_ID}.metainfo.xml"),
    ] {
        // Whole words on lines that run, so a comment naming a path does
        // not count as checking it.
        assert!(
            running
                .iter()
                .any(|line| line.split_whitespace().any(|word| word == installed)),
            "the `flatpak` job never checks the build for {installed}"
        );
    }
    let gone = "dev.postio.Postio.Focus";
    assert!(
        !running.iter().any(|line| line.contains(gone)),
        "the `flatpak` job still looks for {gone}"
    );
}

/// Every SVG the package installs says it is one where a format sniffer
/// looks.
///
/// An image loader does not trust the file extension; it reads the first
/// bytes and asks shared-mime-info what they are. The rule for SVG is `<svg`
/// within the first 257 bytes: `/usr/share/mime/magic` matches it at offset 0
/// with a 256-byte range. A file whose opening tag sits behind a long comment
/// header is, to that sniffer, not an image at all: gdk-pixbuf through glycin
/// says "Couldn't recognize the image file format", GNOME Shell draws nothing
/// where the app icon should be, and `appstreamcli compose` reports a
/// `file-read-error` for the same file. The shipped app icon once carried a
/// 680-byte provenance comment before its `<svg>`, and each of those was
/// diagnosed as something else.
#[test]
fn every_bundled_svg_is_recognised_where_a_sniffer_looks() {
    const SNIFF_WINDOW: usize = 257;

    fn svgs(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|error| panic!("{dir:?}: {error}")) {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                svgs(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "svg") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    svgs(&root().join("crates/postio-widgets/data/icons"), &mut found);
    assert!(!found.is_empty(), "the package should carry the icon SVGs");

    let unrecognised: Vec<&PathBuf> = found
        .iter()
        .filter(|path| {
            let bytes = std::fs::read(path).expect("the icon should be readable");
            let head = &bytes[..bytes.len().min(SNIFF_WINDOW)];
            !head.windows(4).any(|window| window == b"<svg")
        })
        .collect();
    assert!(
        unrecognised.is_empty(),
        "`<svg` is not within the first {SNIFF_WINDOW} bytes of {unrecognised:#?}: an \
         image loader sniffing the content will not recognise these as SVG, and \
         the shell draws a missing icon as nothing"
    );
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}
