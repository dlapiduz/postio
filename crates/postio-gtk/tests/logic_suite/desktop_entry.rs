//! The classic app's desktop integration, while it still builds.
//!
//! Since the package switched to Focus (spec 007, T253) the classic app is
//! built from source only, until it is removed (T256): it has an id and a
//! binary of its own, and no desktop entry. Postio's entry, its icon sizes
//! and the Flatpak that installs them are `postio-focus`'s `packaging.rs`.
//! What is still the classic app's is that its window names itself and
//! draws the bundled icon. Whether the icon renders needs a display and
//! lives in `gtk_window.rs`.

use postio_gtk::{app, resources};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_application_id_is_a_valid_reverse_dns_name() {
    assert_eq!(app::APP_ID, "dev.postio.Postio.Classic");
    assert!(
        gio::Application::id_is_valid(app::APP_ID),
        "GApplication would refuse `{}`",
        app::APP_ID
    );
}

/// `postio` and `dev.postio.Postio` are Focus's. The classic app keeps out
/// of their way and out of the package: no entry is named after its id,
/// and the Flatpak builds none of it.
#[test]
fn the_classic_app_is_not_packaged() {
    let manifest = std::fs::read_to_string(root().join("crates/postio-app/Cargo.toml"))
        .expect("postio-app's manifest");
    assert!(
        manifest.contains(&format!("name = \"{}\"", app::BINARY)),
        "postio-app should build `{}`",
        app::BINARY
    );
    assert!(
        !root()
            .join(format!("crates/postio-focus/data/{}.desktop", app::APP_ID))
            .exists(),
        "the classic app has no desktop entry"
    );
    let flatpak = std::fs::read_to_string(root().join("flatpak/dev.postio.Postio.json"))
        .expect("the Flatpak manifest should be readable");
    for name in ["postio-app", app::BINARY, app::APP_ID] {
        assert!(!flatpak.contains(name), "the Flatpak still names {name}");
    }
}

#[test]
fn the_icon_ships_in_the_bundle() {
    // GTK looks an icon up by name inside an icon-theme directory layout, so
    // the resource path has to be exactly this shape or the lookup silently
    // falls back to `image-missing`.
    let icon = format!("{}/scalable/apps/{}.svg", resources::ICONS, app::ICON_NAME);
    let bundled = resources::walk(resources::ICONS);
    assert!(
        bundled.contains(&icon),
        "the icon `{icon}` the window is drawn with is not in the bundle: {bundled:#?}"
    );

    let bytes = resources::read(&icon).expect("the icon should be readable");
    let svg = String::from_utf8(bytes.to_vec()).expect("the icon should be UTF-8 SVG");
    assert!(svg.contains("<svg"), "the icon should be an SVG");
    assert!(
        svg.contains("viewBox"),
        "the icon needs a viewBox to scale to every size the shell asks for"
    );
}

/// What the window tells the compositor it belongs to.
///
/// GNOME matches a window to an application by the Wayland `app_id`, which
/// GDK takes from `g_get_prgname()` -- the *binary* name unless it is set.
/// Reported against the 0.4.2 Flatpak: the session looked for
/// `postio.desktop`, found nothing, and drew the fallback icon.
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
        "the window will tell the compositor it is {reported:?}, not {}",
        app::APP_ID
    );
}

/// Every bundled SVG says it is one where a format sniffer looks.
///
/// An image loader does not trust the file extension; it reads the first
/// bytes and asks shared-mime-info what they are. The rule for SVG is `<svg`
/// within the first 257 bytes: `/usr/share/mime/magic` matches it at offset 0
/// with a 256-byte range. A file whose opening tag sits behind a long comment header
/// is, to that sniffer, not an image at all: gdk-pixbuf through glycin says
/// "Couldn't recognize the image file format", GNOME Shell logs "Could not
/// load a pixbuf from icon theme" and draws nothing where the app icon
/// should be, and `appstreamcli compose` reports a `file-read-error` for the
/// same file. The shipped app icon carried a 680-byte provenance comment
/// before its `<svg>`, so every one of those happened at once, and each was
/// diagnosed as something else.
#[test]
fn every_bundled_svg_is_recognised_where_a_sniffer_looks() {
    const SNIFF_WINDOW: usize = 257;

    let svgs: Vec<String> = resources::walk(resources::ICONS)
        .into_iter()
        .filter(|path| path.ends_with(".svg"))
        .collect();
    assert!(!svgs.is_empty(), "the bundle should carry the icon SVGs");

    let unrecognised: Vec<String> = svgs
        .iter()
        .filter(|path| {
            let bytes = resources::read(path).expect("the icon should be readable");
            let head = &bytes[..bytes.len().min(SNIFF_WINDOW)];
            !head.windows(4).any(|w| w == b"<svg")
        })
        .cloned()
        .collect();

    assert!(
        unrecognised.is_empty(),
        "`<svg` is not within the first {SNIFF_WINDOW} bytes of {unrecognised:#?}: an \
         image loader sniffing the content will not recognise these as SVG, and \
         the shell draws a missing icon as nothing"
    );
}
