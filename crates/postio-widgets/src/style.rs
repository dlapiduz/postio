//! The shared stylesheet `data/widgets.css`: every value is a `--postio-*` role the app defines;
//! metrics come from `postio_ui::tokens` via `build.rs`.
//! GTK compares specificity only within one provider, so an app layering over these rules must
//! `@import url("resource:///dev/postio/Widgets/widgets.css");` in its own sheet, after `register`
//! runs.
//! `install` is for an app with no sheet of its own.

use std::sync::OnceLock;

use gtk::{gdk, gio, glib};

pub const WIDGETS_CSS: &str = "/dev/postio/Widgets/widgets.css";

/// Icons live at `scalable/apps/<name>.svg` beneath this resource path.
const ICONS: &str = "/dev/postio/Widgets/icons";

const BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/postio-widgets.gresource"));

/// Safe to call more than once; touches neither the filesystem nor the network.
pub fn register() {
    static REGISTERED: OnceLock<()> = OnceLock::new();
    REGISTERED.get_or_init(|| {
        let bytes = glib::Bytes::from_static(BUNDLE);
        let resource = gio::Resource::from_data(&bytes)
            .expect("the compiled GResource bundle is malformed; this is a build bug");
        gio::resources_register(&resource);
    });
}

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

/// For an app with no sheet of its own; otherwise import this sheet into it (see module docs).
pub fn install(display: &gdk::Display) -> gtk::CssProvider {
    register();
    let provider = gtk::CssProvider::new();
    provider.connect_parsing_error(|_, section, error| {
        // A parse error means a rule was dropped; loud because the controls would be subtly wrong.
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

    const SHEET: &str = include_str!("../data/widgets.css");

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
