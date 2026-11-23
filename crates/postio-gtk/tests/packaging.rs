//! The package is Postio: one app, one binary, one application id, one desktop entry and one
//! AppStream component, in the desktop Flatpak.
//!
//! POSTIO-OWN-BINARY: plain libtest, and `focus_suite` is a `harness = false` GTK harness.
//! None of this needs a display; each part is checked against the others.

use std::path::{Path, PathBuf};

use postio_gtk::app::{APP_ID, ICON_NAME};

const BINARY: &str = "postio";
const HELPER: &str = "postio-extract-helper";
const MCP: &str = "postio-mcp";

const MANIFEST: &str = "flatpak/dev.postio.Postio.json";

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

fn commands(manifest: &str) -> Vec<&str> {
    manifest
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('"'))
        .collect()
}

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
    assert_eq!(value("StartupWMClass"), APP_ID);
    assert_eq!(value("Icon"), ICON_NAME);
}

#[test]
fn postio_handles_mailto_and_postio_links() {
    let types = list("MimeType");
    for wanted in ["x-scheme-handler/mailto", "x-scheme-handler/postio"] {
        assert!(
            types.iter().any(|kind| kind == wanted),
            "the entry should register {wanted}, got {types:?}"
        );
    }
    let exec = value("Exec");
    assert!(
        exec.contains("%u") || exec.contains("%U"),
        "Exec should pass the link on, got `{exec}`"
    );
}

/// A second entry would be a second icon in the app grid, and two entries claiming one scheme leave
/// the choice to whichever the desktop reads first.
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

/// 16 and 32 are hand-drawn rather than downscales; 48, 64 and 128 are what GNOME asks for at 1x. A
/// size with no file is drawn as nothing, not a fallback.
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

#[test]
fn the_desktop_flatpak_builds_and_installs_postio_its_helper_and_the_agent_entry_only() {
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
    let built = [
        format!("--package postio-gtk --bin {BINARY}"),
        format!("--package postio-session --bin {HELPER}"),
        format!("--package postio-mcp --bin {MCP}"),
    ];
    assert!(
        builds.len() == built.len()
            && built
                .iter()
                .all(|one| builds.iter().any(|line| line.contains(one.as_str()))),
        "the manifest should build {built:?} and nothing else, got {builds:#?}"
    );
    for name in [BINARY, HELPER, MCP] {
        let binary = format!("target/release/{name} /app/bin/{name}\"");
        assert!(
            commands.iter().any(|line| line.contains(&binary)),
            "the manifest never installs /app/bin/{name}"
        );
    }
    let installed_binaries: Vec<&&str> = commands
        .iter()
        .filter(|line| line.contains("/app/bin/"))
        .collect();
    assert_eq!(
        installed_binaries.len(),
        3,
        "the manifest installs a binary besides {BINARY}, {HELPER} and {MCP}: \
         {installed_binaries:#?}"
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
    assert!(
        !metainfo.contains("Postio Focus"),
        "the metainfo still describes Focus as a second app"
    );
}

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

#[test]
fn the_release_checks_the_bundle_carries_postio() {
    let workflow = read(".github/workflows/release.yml");
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
        format!("files/bin/{MCP}"),
        format!("files/share/applications/{APP_ID}.desktop"),
        format!("files/share/metainfo/{APP_ID}.metainfo.xml"),
    ] {
        // Whole words on lines that run, so a comment naming a path does not count as checking it.
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
    // The same job builds the terminal bundle, which carries `postio-tui` and no desktop entry.
    assert!(
        running.iter().any(|line| line
            .split_whitespace()
            .any(|word| word == "files/bin/postio-tui")),
        "the `flatpak` job never checks the terminal bundle for files/bin/postio-tui"
    );
    assert!(
        running
            .iter()
            .any(|line| line.contains("dev.postio.PostioTui")),
        "the `flatpak` job has no list of what dev.postio.PostioTui carries"
    );
}

/// An image loader sniffs for `<svg` within the first 257 bytes (shared-mime-info): an opening tag
/// behind a long comment header is not an image to it, and the shipped app icon once carried a
/// 680-byte comment before its `<svg>`.
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

#[test]
fn both_packages_may_read_the_gnupg_keyring_and_never_write_it() {
    for manifest in [MANIFEST, "flatpak/dev.postio.PostioTui.json"] {
        let text = read(manifest);
        let grants: Vec<&str> = text
            .lines()
            .map(|line| line.trim().trim_end_matches(',').trim_matches('"'))
            .filter(|arg| arg.contains(".gnupg"))
            .collect();
        assert_eq!(
            grants,
            ["--filesystem=~/.gnupg:ro"],
            "{manifest}: public keys are read in place, never written"
        );
    }
}
