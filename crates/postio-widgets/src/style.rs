//! The shared widgets' stylesheet, `data/widgets.css`, and how an app loads
//! it.
//!
//! Every value in it is a `--postio-*` role, and each app defines the roles
//! (specs/007-postio-focus research R11): the classic app from its generated
//! `tokens.css`, and Focus from libadwaita's own colours.
//!
//! # One sheet with the app's own, not a second provider
//!
//! GTK compares specificity only inside one `GtkCssProvider`: across two
//! providers at one priority, the one added later wins every property it
//! sets, whatever its selectors. So an app that dresses a surface's controls
//! over these rules must layer them in the *same* provider, which is what
//! `@import url("resource:///dev/postio/Widgets/widgets.css");` at the top of
//! its own sheet does -- GTK parses an imported sheet into the importing
//! provider, at the point of the import. [`register`] makes the URL resolve,
//! and must run before that sheet is parsed. [`install`] is for an app with
//! no sheet of its own to layer.

use std::sync::OnceLock;

use gtk::{gdk, gio, glib};

/// Where the shared stylesheet lives in the bundle, as a resource path.
pub const WIDGETS_CSS: &str = "/dev/postio/Widgets/widgets.css";

/// The shared stylesheet as an `@import` names it.
pub const WIDGETS_CSS_URL: &str = "resource:///dev/postio/Widgets/widgets.css";

/// The bundled app icon, laid out as `GtkIconTheme` expects a resource path:
/// `scalable/apps/<name>.svg` beneath this directory.
pub const ICONS: &str = "/dev/postio/Widgets/icons";

const BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/postio-widgets.gresource"));

/// Register the bundle with GIO. Safe to call more than once; never touches
/// the filesystem or the network.
pub fn register() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        let bytes = glib::Bytes::from_static(BUNDLE);
        let resource = gio::Resource::from_data(&bytes)
            .expect("the compiled GResource bundle is malformed; this is a build bug");
        gio::resources_register(&resource);
    });
}

/// Make the bundled app icon resolvable by name on `display`. Idempotent: a
/// path already on the theme is not added again.
pub fn install_icons(display: &gdk::Display) {
    register();
    let theme = gtk::IconTheme::for_display(display);
    let already = theme
        .resource_path()
        .iter()
        .any(|path| path.as_str() == ICONS);
    if !already {
        theme.add_resource_path(ICONS);
    }
}

/// Load the shared stylesheet for `display` as a provider of its own, at
/// application priority, and return it.
///
/// For an app with no sheet of its own to layer over these rules; one that
/// has one imports this sheet into it instead (see the module docs).
pub fn install(display: &gdk::Display) -> gtk::CssProvider {
    register();
    let provider = gtk::CssProvider::new();
    provider.connect_parsing_error(|_, section, error| {
        // A parse error means the sheet used something GTK's CSS subset does
        // not have, and a rule was dropped: loud, because the controls would
        // be subtly wrong.
        glib::g_critical!("postio", "widgets.css: {}: {error}", section.to_str());
    });
    provider.load_from_resource(WIDGETS_CSS);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    provider
}

#[cfg(test)]
mod tests {
    //! The text guards postio-gtk keeps over `shell.css`, over this sheet:
    //! these rules were under them while they lived there. Each check is
    //! shown failing on a sheet that breaks it, so a pass means something.

    const SHEET: &str = include_str!("../data/widgets.css");

    /// Every transition over the motion budget (CLAUDE.md: <= 100ms or
    /// absent), as `line: duration`.
    fn over_budget(css: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (number, line) in css.lines().enumerate() {
            let line = line.trim();
            if line.starts_with("/*") || line.starts_with('*') || !line.contains("transition") {
                continue;
            }
            for token in line.split([' ', ',', ':', ';']) {
                let ms = token
                    .strip_suffix("ms")
                    .and_then(|ms| ms.parse::<f64>().ok())
                    .or_else(|| {
                        token
                            .strip_suffix('s')
                            .and_then(|s| s.parse::<f64>().ok())
                            .map(|s| s * 1000.0)
                    });
                if let Some(ms) = ms.filter(|ms| *ms > 100.0) {
                    out.push(format!("{}: {ms}ms", number + 1));
                }
            }
        }
        out
    }

    /// Every type role whose size is typed in rather than named.
    fn retyped_roles(css: &str) -> Vec<&'static str> {
        postio_ui::tokens::TYPE_ROLES
            .iter()
            .filter(|(_, size)| css.contains(&format!("font-size: {size};")))
            .map(|(role, _)| *role)
            .collect()
    }

    #[test]
    fn nothing_in_the_sheet_outruns_the_motion_budget() {
        assert_eq!(
            over_budget(".a {\n  transition: opacity 250ms;\n}"),
            ["2: 250ms"],
            "the check must catch a slow transition"
        );
        assert!(
            over_budget(SHEET).is_empty(),
            "widgets.css outruns the motion budget: {:?}",
            over_budget(SHEET)
        );
    }

    #[test]
    fn a_type_role_is_named_not_retyped() {
        let (role, size) = postio_ui::tokens::TYPE_ROLES[0];
        assert_eq!(
            retyped_roles(&format!(".a {{ font-size: {size}; }}")),
            [role],
            "the check must catch a retyped role"
        );
        assert!(
            retyped_roles(SHEET).is_empty(),
            "widgets.css retypes these roles' sizes instead of var(--postio-text-…): {:?}",
            retyped_roles(SHEET)
        );
    }
}
