//! The compiled GResource bundle: the generated stylesheet and the vendored
//! fonts, linked into the binary by `build.rs`.
//!
//! Registration is idempotent and never touches the filesystem or the network.

use std::sync::OnceLock;

/// Where everything in the bundle lives.
pub const PREFIX: &str = "/dev/postio/Postio";

/// The generated design tokens.
pub const TOKENS_CSS: &str = "/dev/postio/Postio/tokens.css";

/// The directory holding the vendored font families, each next to its licence.
pub const FONTS: &str = "/dev/postio/Postio/fonts";

/// The bundled icon theme, laid out the way `GtkIconTheme` expects a resource
/// path to be: `<size>/<context>/<name>.svg` beneath this directory.
pub const ICONS: &str = "/dev/postio/Postio/icons";

static REGISTERED: OnceLock<()> = OnceLock::new();

const BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/postio.gresource"));

/// Register the bundle with GIO. Safe to call more than once.
pub fn register() {
    REGISTERED.get_or_init(|| {
        let bytes = glib::Bytes::from_static(BUNDLE);
        let resource = gio::Resource::from_data(&bytes)
            .expect("the compiled GResource bundle is malformed; this is a build bug");
        gio::resources_register(&resource);
    });
}

/// Every file in the bundle under `path`, depth first, as full resource paths.
pub fn walk(path: &str) -> Vec<String> {
    register();
    let mut out = Vec::new();
    let mut stack = vec![path.trim_end_matches('/').to_string()];
    while let Some(dir) = stack.pop() {
        let children = match gio::resources_enumerate_children(
            &format!("{dir}/"),
            gio::ResourceLookupFlags::NONE,
        ) {
            Ok(children) => children,
            Err(_) => continue,
        };
        for child in children {
            let child = child.to_string();
            if let Some(sub) = child.strip_suffix('/') {
                stack.push(format!("{dir}/{sub}"));
            } else {
                out.push(format!("{dir}/{child}"));
            }
        }
    }
    out.sort();
    out
}

/// Read one file out of the bundle.
pub fn read(path: &str) -> Result<glib::Bytes, glib::Error> {
    register();
    gio::resources_lookup_data(path, gio::ResourceLookupFlags::NONE)
}
