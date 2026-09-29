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
    // The import first: CSS reads an `@import` only before the first rule.
    provider.load_from_string(&format!("{SURFACES}\n{COLOURS}"));
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    INSTALLED.with(|installed| installed.borrow_mut().push(display.clone()));
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
}
