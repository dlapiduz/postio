//! The checks that need a real GTK display: that the generated stylesheet
//! parses in GTK's CSS subset, that every token resolves in light, dark and
//! high-contrast, and that the embedded fonts reach a widget.
//!
//! GTK is single-threaded and single-init, so this is deliberately *one* test
//! function running the whole suite in order. If there is no display — a
//! headless CI runner without `xvfb` — it skips rather than failing, and says
//! so; run it locally, or under `xvfb-run`, to exercise the real thing.
//!
//! Nothing here touches the network: the stylesheet and the fonts both come
//! out of the binary's own GResource bundle.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::{fonts, resources, style};

/// A colour no token has, used to spot a `var()` that did not resolve: an
/// unresolvable variable drops the declaration, so the probe label inherits
/// this from its window instead.
const SENTINEL: Rgba = Rgba(1.0, 0.0, 1.0, 1.0);

/// Variables libadwaita derives from ours. Probed too, because the point of
/// mapping onto the named colours is that stock widgets keep working.
const DERIVED: [&str; 1] = ["--accent-color"];

pub fn the_generated_stylesheet_works_in_gtk() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();

    // ── the sheet parses in GTK's CSS subset ──────────────────────────────
    resources::register();
    let errors: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let provider = gtk::CssProvider::new();
    provider.connect_parsing_error({
        let errors = errors.clone();
        move |_, section, error| {
            errors
                .borrow_mut()
                .push(format!("{}: {error}", section.to_str()));
        }
    });
    provider.load_from_resource(resources::TOKENS_CSS);
    assert!(
        errors.borrow().is_empty(),
        "tokens.css does not parse in GTK's CSS subset: {:#?}",
        errors.borrow()
    );

    // ── the fonts come out of the binary ──────────────────────────────────
    // Registered before any widget is built: a PangoContext keeps the family
    // it has already resolved.
    let installed = fonts::install().expect("the embedded fonts should install");
    assert_eq!(installed.len(), 8, "eight faces ship in the bundle");
    for path in &installed {
        assert!(path.exists(), "{} was not unpacked", path.display());
    }

    let font_map = pangocairo::FontMap::default();
    let families: Vec<String> = font_map
        .list_families()
        .iter()
        .map(|f| f.name().to_string())
        .collect();
    for wanted in fonts::FAMILIES {
        assert!(
            families.iter().any(|f| f == wanted),
            "`{wanted}` is not in the font map; the bundle did not register"
        );
    }

    let licenses = fonts::licenses();
    assert_eq!(licenses.len(), 3, "every family ships its licence");
    for (family, text) in &licenses {
        assert!(
            text.contains("SIL OPEN FONT LICENSE"),
            "{family} is missing its OFL text"
        );
    }

    // ── install for real, then read every token back out of GTK ───────────
    style::install(&display);

    let mut names = token_names();
    assert!(
        names.len() > 40,
        "expected the full token set, got {} names",
        names.len()
    );
    names.extend(DERIVED.iter().map(|s| s.to_string()));

    let light = snapshot(&display, &names, false, false);
    let dark = snapshot(&display, &names, true, false);
    let light_hc = snapshot(&display, &names, false, true);
    let dark_hc = snapshot(&display, &names, true, true);

    // Lengths, shadows and font stacks are not colours and cannot be probed
    // this way: that the sheet parses proves they are well formed, and the
    // type-role check below proves the font stacks reach a widget.
    for (scheme, values) in [
        ("light", &light),
        ("dark", &dark),
        ("high contrast", &light_hc),
        ("dark high contrast", &dark_hc),
    ] {
        for name in names.iter().filter(|n| is_colour_token(n)) {
            assert_ne!(
                values[name], SENTINEL,
                "`{name}` does not resolve in the {scheme} scheme"
            );
        }
    }

    // ── the design's own rules, where they matter ─────────────────────────
    assert_eq!(
        light["--postio-accent"].bytes(),
        (89, 128, 166),
        "the steel accent should survive into GTK"
    );

    // Hairlines lift on the dark ground instead of darkening.
    assert!(
        over(dark["--postio-hairline"], dark["--postio-ground"]) > luma(dark["--postio-ground"]),
        "the dark hairline should be lighter than the dark ground"
    );
    assert!(
        over(light["--postio-hairline"], light["--postio-ground"]) < luma(light["--postio-ground"]),
        "the light hairline should still be ink on paper"
    );

    // Steel goes light-on-dark.
    assert!(
        luma(dark["--postio-accent"]) > luma(light["--postio-accent"]),
        "the dark scheme should take the accent's light step"
    );
    // ...and libadwaita derives its own standalone accent from ours, per
    // scheme, so stock widgets follow without a second hard-coded value.
    assert!(
        luma(dark["--accent-color"]) > luma(light["--accent-color"]),
        "libadwaita should derive a light-on-dark standalone accent from ours"
    );

    // The selected row sits on the accent's deep step in dark.
    assert_eq!(
        dark["--postio-selected-bg"].bytes(),
        (29, 45, 61),
        "dark selection should be the accent's deep step"
    );

    // High contrast only tightens: same ground, more separation.
    assert_eq!(light_hc["--postio-ground"], light["--postio-ground"]);
    assert!(
        contrast(light_hc["--postio-hairline"], light_hc["--postio-ground"])
            > contrast(light["--postio-hairline"], light["--postio-ground"]),
        "high contrast should make the hairline more visible, not less"
    );
    assert!(
        contrast(dark_hc["--postio-dim"], dark_hc["--postio-ground"])
            > contrast(dark["--postio-dim"], dark["--postio-ground"]),
        "high contrast should bring dimmed text back up on a dark ground"
    );

    // ── the type roles reach real widgets ─────────────────────────────────
    let window = gtk::Window::new();
    style::track(&window);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    window.set_child(Some(&row));

    let mut labels = Vec::new();
    for class in ["postio-heading", "postio-body", "postio-mono"] {
        let label = gtk::Label::new(Some("Inbox — maildir index rebuild"));
        if class != "postio-body" {
            // Body text is the inherited default, not a class.
            label.add_css_class(class);
        }
        row.append(&label);
        labels.push((class, label));
    }
    window.present();
    pump();

    let mut widths = HashMap::new();
    for (class, label) in &labels {
        let context = label.pango_context();
        let description = context.font_description().expect("a font description");
        let font = context.load_font(&description).expect("a loaded font");
        let family = font
            .face()
            .map(|f| f.family().name().to_string())
            .unwrap_or_default();
        let expected = match *class {
            "postio-heading" => "Barlow Condensed",
            "postio-mono" => "IBM Plex Mono",
            _ => "Barlow",
        };
        assert_eq!(
            family, expected,
            "`{class}` should render in {expected}, not {family}"
        );
        widths.insert(*class, label.measure(gtk::Orientation::Horizontal, -1).1);
    }

    assert!(
        widths["postio-heading"] < widths["postio-body"],
        "Barlow Condensed should set the same string narrower than Barlow"
    );

    window.destroy();
}

/// Resolve every token in one scheme and read the answers back out of GTK.
///
/// The scheme is set two ways at once, exactly as the app does it: the style
/// manager (so libadwaita's own derived colours follow) and the classes
/// `tokens.css` keys its blocks off (so ours do).
fn snapshot(
    display: &gdk::Display,
    names: &[String],
    dark: bool,
    high_contrast: bool,
) -> HashMap<String, Rgba> {
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(if dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });
    pump();

    let probed: Vec<&String> = names.iter().filter(|n| is_colour_token(n)).collect();

    let mut css = format!(
        ":root {{ color: rgba({}, {}, {}, {}); }}\n",
        (SENTINEL.0 * 255.0) as u8,
        (SENTINEL.1 * 255.0) as u8,
        (SENTINEL.2 * 255.0) as u8,
        SENTINEL.3
    );
    for (i, name) in probed.iter().enumerate() {
        css.push_str(&format!("label.probe{i} {{ color: var({name}); }}\n"));
    }

    let provider = gtk::CssProvider::new();
    provider.connect_parsing_error(|_, section, error| {
        panic!(
            "the probe sheet failed to parse at {}: {error}",
            section.to_str()
        )
    });
    provider.load_from_string(&css);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );

    let window = gtk::Window::new();
    if dark {
        window.add_css_class(style::DARK_CLASS);
    }
    if high_contrast {
        window.add_css_class(style::HIGH_CONTRAST_CLASS);
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.set_child(Some(&column));

    let mut probes = Vec::new();
    for (i, _) in probed.iter().enumerate() {
        let label = gtk::Label::new(Some("x"));
        label.add_css_class(&format!("probe{i}"));
        column.append(&label);
        probes.push(label);
    }
    window.present();
    pump();

    let values = probed
        .iter()
        .map(|n| (*n).clone())
        .zip(probes.iter().map(|l| Rgba::from(l.color())))
        .collect();

    window.destroy();
    gtk::style_context_remove_provider_for_display(display, &provider);
    values
}

/// Every variable the generated sheet defines: the `--postio-*` roles and the
/// libadwaita named colours it overrides.
fn token_names() -> Vec<String> {
    let bytes = resources::read(resources::TOKENS_CSS).expect("the bundle carries tokens.css");
    let css = String::from_utf8_lossy(&bytes).into_owned();
    let mut names: Vec<String> = css
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("--"))
        .filter_map(|l| l.split(':').next())
        .map(|n| n.trim().to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn is_colour_token(name: &str) -> bool {
    !(name.starts_with("--postio-space-")
        || name.starts_with("--postio-radius-")
        || name.starts_with("--postio-shadow-")
        || name.starts_with("--postio-font-"))
}

fn pump() {
    for _ in 0..50 {
        glib::MainContext::default().iteration(false);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgba(f32, f32, f32, f32);

impl From<gdk::RGBA> for Rgba {
    fn from(c: gdk::RGBA) -> Self {
        Rgba(c.red(), c.green(), c.blue(), c.alpha())
    }
}

impl Rgba {
    fn bytes(self) -> (u8, u8, u8) {
        (
            (self.0 * 255.0).round() as u8,
            (self.1 * 255.0).round() as u8,
            (self.2 * 255.0).round() as u8,
        )
    }
}

fn luma(c: Rgba) -> f32 {
    0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2
}

/// A hairline is drawn *over* the ground, so its alpha is part of what you
/// see; compare composited luminance, not the raw colour.
fn over(fg: Rgba, bg: Rgba) -> f32 {
    let mix = |f: f32, b: f32| fg.3 * f + (1.0 - fg.3) * b;
    luma(Rgba(mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2), 1.0))
}

fn contrast(fg: Rgba, bg: Rgba) -> f32 {
    (over(fg, bg) - luma(bg)).abs()
}
