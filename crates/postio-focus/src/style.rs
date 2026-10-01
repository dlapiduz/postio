//! Focus's stylesheet: its colour roles, and its own surfaces over the
//! shared widgets' rules (research R11).
//!
//! The shared widgets read `--postio-*` roles and each app defines them.
//! Focus defines the colour roles from libadwaita's own named colours
//! (`data/focus-colours.css`), so the system's accent and its light and dark
//! arrive through `AdwStyleManager` with nothing of Focus's in between. Its
//! surfaces are `data/focus.css`, which imports the shared sheet at its top so
//! both are one provider: GTK compares specificity only inside one.
//!
//! **The accent is reserved** (FR-091): action markers, the keyboard focus
//! ring and the has-action toggle, and nothing else. The shared sheet uses
//! the accent for a primary button's fill and a few hovers, which is the
//! classic app's language; Focus's sheet re-dresses those, and a test here
//! reads both sheets as GTK would and fails if any other rule paints with it.

/// Focus's colour roles.
pub const COLOURS: &str = include_str!("../data/focus-colours.css");

/// Focus's own surfaces, the shared sheet imported at the top.
pub const SURFACES: &str = include_str!("../data/focus.css");

/// Load Focus's stylesheet for `display`, once: the shared sheet, Focus's
/// surfaces over it and its colour roles, as one provider at application
/// priority. A second call for the same display does nothing.
pub fn install(display: &gtk::gdk::Display) {
    thread_local! {
        static INSTALLED: std::cell::RefCell<Vec<gtk::gdk::Display>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    if INSTALLED.with(|installed| installed.borrow().contains(display)) {
        return;
    }
    // The shared sheet's `resource://` URL has to resolve before the
    // `@import` naming it is parsed.
    postio_widgets::style::register();
    let provider = gtk::CssProvider::new();
    provider.connect_parsing_error(|_, section, error| {
        // A rule GTK's CSS could not read is a rule that was dropped: loud,
        // because the screen would be subtly wrong.
        gtk::glib::g_critical!("postio-focus", "focus.css: {}: {error}", section.to_str());
    });
    // A media query in these sheets is read against the provider's own
    // scheme (GTK 4.20), not the system's: it follows AdwStyleManager here,
    // so `prefers-color-scheme: dark` holds exactly when Focus is dark -- the
    // message dialog's palette depends on it (focus-colours.css).
    let manager = adw::StyleManager::for_display(display);
    provider.set_prefers_color_scheme(scheme(manager.is_dark()));
    manager.connect_dark_notify({
        let provider = provider.clone();
        move |manager| provider.set_prefers_color_scheme(scheme(manager.is_dark()))
    });
    // The import first: CSS reads an `@import` only before the first rule.
    provider.load_from_string(&format!("{SURFACES}\n{COLOURS}"));
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    INSTALLED.with(|installed| installed.borrow_mut().push(display.clone()));
}

/// The scheme a media query sees.
fn scheme(dark: bool) -> gtk::InterfaceColorScheme {
    if dark {
        gtk::InterfaceColorScheme::Dark
    } else {
        gtk::InterfaceColorScheme::Light
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared widgets' sheet, which `SURFACES` imports.
    const SHARED: &str = include_str!("../../postio-widgets/data/widgets.css");

    /// Every declaration in `css`, in order: `(selector, property, value)`,
    /// one per selector of a group. Comments and at-rules are skipped; GTK's
    /// CSS has no nesting to follow.
    fn declarations(css: &str) -> Vec<(String, String, String)> {
        let mut text = String::new();
        let mut rest = css;
        while let Some(start) = rest.find("/*") {
            text.push_str(&rest[..start]);
            rest = rest[start + 2..]
                .split_once("*/")
                .map_or("", |(_, after)| after);
        }
        text.push_str(rest);
        let mut found = Vec::new();
        for block in text.split('}') {
            let Some((selectors, body)) = block.split_once('{') else {
                continue;
            };
            let selectors: Vec<String> = selectors
                .lines()
                .filter(|line| !line.trim_start().starts_with('@'))
                .collect::<Vec<_>>()
                .join(" ")
                .split(',')
                .map(|selector| selector.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|selector| !selector.is_empty())
                .collect();
            for declaration in body.split(';') {
                let Some((property, value)) = declaration.split_once(':') else {
                    continue;
                };
                for selector in &selectors {
                    found.push((
                        selector.clone(),
                        property.trim().to_owned(),
                        value.trim().to_owned(),
                    ));
                }
            }
        }
        found
    }

    /// Where the accent still paints once both sheets are read as one: the
    /// last word on each selector's property, when it names the accent.
    /// Defining a role is not painting, so custom properties are left out.
    fn painted_with_accent(sheets: &[&str]) -> Vec<String> {
        let mut last: Vec<((String, String), String)> = Vec::new();
        for sheet in sheets {
            for (selector, property, value) in declarations(sheet) {
                if property.starts_with("--") {
                    continue;
                }
                let key = (selector, property);
                last.retain(|(held, _)| *held != key);
                last.push((key, value));
            }
        }
        last.into_iter()
            .filter(|(_, value)| value.contains("accent"))
            .map(|((selector, property), _)| format!("{selector} {{ {property} }}"))
            .collect()
    }

    /// The three places FR-091 gives the accent. The list's cursor is its
    /// focus ring: GTK marks the row the keyboard is on `:selected`, and
    /// Focus draws that as the ring, never as a fill -- in the inbox, and in
    /// the full views drawn in Filtered's frame (Filtered, the rules list).
    fn reserved(rule: &str) -> bool {
        rule.contains(":focus")
            || rule.starts_with(".focus-list > row:selected ")
            || rule.starts_with(".focus-filtered-list > row:selected ")
            || rule.contains(".focus-marker")
            || rule.contains(".focus-has-action:checked")
    }

    #[test]
    fn the_check_sees_an_accent_rule_and_a_later_one_that_covers_it() {
        let painted = painted_with_accent(&[
            ".a { color: var(--postio-accent); }\n.b:focus-visible { outline-color: var(--postio-accent); }",
            ".a { color: var(--postio-ink); }",
        ]);
        assert_eq!(painted, [".b:focus-visible { outline-color }"]);
    }

    #[test]
    fn the_accent_paints_only_markers_the_focus_ring_and_the_has_action_toggle() {
        let stray: Vec<String> = painted_with_accent(&[SHARED, COLOURS, SURFACES])
            .into_iter()
            .filter(|rule| !reserved(rule))
            .collect();
        assert!(
            stray.is_empty(),
            "FR-091 keeps the accent for action markers, the focus ring and the \
             has-action toggle; these rules still paint with it in Focus: {stray:#?}"
        );
    }

    /// A selector's specificity, in this stylesheet's own narrow subset: one
    /// compound selector, no combinators -- an optional leading type, then
    /// any number of `.class` or `:pseudo-class` parts, which CSS weighs the
    /// same. `(classes, types)`, compared as a tuple: a selector with more
    /// classes always outweighs one with more types, whatever the counts.
    fn specificity(selector: &str) -> (u32, u32) {
        let mut rest = selector;
        let mut types = 0;
        if let Some(end) = rest.find(['.', ':']) {
            if end > 0 {
                types = 1;
            }
            rest = &rest[end..];
        } else if !rest.is_empty() {
            types = 1;
            rest = "";
        }
        let mut classes = 0;
        while let Some(marker) = rest.chars().next() {
            debug_assert!(marker == '.' || marker == ':');
            let end = rest[1..].find(['.', ':']).map_or(rest.len(), |i| i + 1);
            classes += 1;
            rest = &rest[end..];
        }
        (classes, types)
    }

    /// Whether `selector` (as [`declarations`] parsed it) matches a `button`
    /// wearing every one of `classes`, `:hover` or not. `false` for a
    /// selector this narrow matcher cannot parse -- a combinator, a
    /// pseudo-element -- so a candidate this helper misjudges is at least
    /// never counted as a match it should not be.
    fn matches_button(selector: &str, classes: &[&str], hovered: bool) -> bool {
        let Some(mut rest) = selector.strip_prefix("button") else {
            return false;
        };
        let mut wants_hover = false;
        while let Some(marker) = rest.chars().next() {
            let end = rest[1..].find(['.', ':']).map_or(rest.len(), |i| i + 1);
            let part = &rest[1..end];
            match marker {
                '.' if !classes.contains(&part) => return false,
                ':' if part == "hover" => wants_hover = true,
                '.' | ':' => {}
                _ => return false,
            }
            rest = &rest[end..];
        }
        wants_hover == hovered || !wants_hover
    }

    /// The `border-radius` a `button` wearing every one of `classes` ends up
    /// with once every declaration in `sheets` (in order) has cascaded: the
    /// most specific match wins, and a tie goes to whichever comes later --
    /// GTK's own rule, applied here in text rather than on a live widget, so
    /// this needs no display (T173).
    fn winning_border_radius(sheets: &[&str], classes: &[&str], hovered: bool) -> Option<String> {
        let mut winner: Option<((u32, u32), String)> = None;
        for sheet in sheets {
            for (selector, property, value) in declarations(sheet) {
                if property != "border-radius" || !matches_button(&selector, classes, hovered) {
                    continue;
                }
                let spec = specificity(&selector);
                if winner.as_ref().is_none_or(|(held, _)| spec >= *held) {
                    winner = Some((spec, value));
                }
            }
        }
        winner.map(|(_, value)| value)
    }

    #[test]
    fn specificity_weighs_a_class_over_any_number_of_types() {
        assert_eq!(specificity("button"), (0, 1));
        assert_eq!(specificity(".circular"), (1, 0));
        assert_eq!(specificity("button.circular"), (1, 1));
        assert_eq!(specificity("button.postio-icon-button.circular"), (2, 1));
        assert_eq!(specificity("button.postio-icon-button:hover"), (2, 1));
    }

    /// T173: the top bar's close button (`chrome.rs`) wears
    /// `postio-icon-button`, `focus-close` and `circular`. Its `circular`
    /// class is a promise -- a round hit target -- and the shared sheet's
    /// plain `button.postio-icon-button` must not outrank it merely by
    /// pairing a class with a type selector.
    #[test]
    fn a_circular_icon_button_keeps_its_round_radius_at_rest_and_on_hover() {
        let classes = ["postio-icon-button", "focus-close", "circular"];
        let resting = winning_border_radius(&[SHARED, SURFACES], &classes, false);
        let hovered = winning_border_radius(&[SHARED, SURFACES], &classes, true);
        assert_eq!(
            resting, hovered,
            "the close button's radius should not change on hover"
        );
        assert_ne!(
            resting.as_deref(),
            Some("var(--postio-radius-sm)"),
            "the icon button's plain radius still outranks `circular`: {resting:?}"
        );
    }
}
