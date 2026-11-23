//! Focus's stylesheet. Colour roles come from libadwaita's named colours
//! (`data/focus-colours.css`); surfaces are `data/focus.css`, which imports the shared sheet
//! at its top so both are one provider: GTK compares specificity only inside one. **The accent is
//! reserved**: action markers, the keyboard focus ring and the
//! has-action toggle, and nothing else; a test reads both sheets as GTK would and fails if any
//! other rule paints with it.

pub const COLOURS: &str = include_str!("../data/focus-colours.css");

pub const SURFACES: &str = include_str!("../data/focus.css");

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
        gtk::glib::g_critical!("postio-gtk", "focus.css: {}: {error}", section.to_str());
    });
    // A media query is read against the provider's own scheme (GTK 4.20), not the system's: it
    // follows AdwStyleManager, so `prefers-color-scheme: dark` holds exactly when Focus is dark.
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

    const SHARED: &str = include_str!("../../postio-widgets/data/widgets.css");

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

    /// The accent's allowed places: the list's cursor ring, an unread dot, the thread bar's current
    /// chip, and the invisible probe widgets whose colour hands a body its link colour.
    fn reserved(rule: &str) -> bool {
        rule == ".focus-open .postio-flow-accent { color }"
            || rule == ".focus-compose .postio-flow-accent { color }"
            || rule == ".focus-compose-surface .postio-flow-accent { color }"
            || rule.contains(":focus")
            || rule.starts_with(".focus-list > row:selected ")
            || rule.starts_with(".focus-filtered-list > row:selected ")
            || rule.starts_with(".focus-contacts-list > row:selected ")
            || rule.starts_with(".focus-results-row.focus-results-cursor ")
            || rule.starts_with(".focus-results-child.focus-results-cursor ")
            || rule == ".focus-results-gutter.focus-results-dot { color }"
            || rule.starts_with(".focus-file-card.focus-file-focused ")
            || rule.starts_with(".focus-quicklook-card.focus-quicklook-current ")
            || rule.starts_with("button.focus-nothing-row.focus-nothing-focused ")
            || rule.starts_with(".focus-digest-summary-statement.focus-digest-summary-focused ")
            || rule.starts_with(".focus-thread-chip-current ")
            || rule.starts_with(".focus-new-reply-banner ")
            || rule.starts_with(".focus-older-reply ")
            || rule.starts_with(".focus-forward-card-selected ")
            || rule.starts_with(".focus-outline-current ")
            || rule == ".focus-thread-dot { color }"
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

    fn winning(sheets: &[&str], selector: &str, property: &str) -> Option<String> {
        sheets
            .iter()
            .flat_map(|sheet| declarations(sheet))
            .filter(|(found, name, _)| found == selector && name == property)
            .map(|(_, _, value)| value)
            .next_back()
    }

    #[test]
    fn the_places_list_marks_its_highlighted_row_in_the_bar_s_neutral_ground() {
        let sheets = [SHARED, COLOURS, SURFACES];
        let bar = winning(
            &sheets,
            ".focus-bar-results > row:selected",
            "background-color",
        );
        assert_eq!(bar.as_deref(), Some("var(--postio-selected-bg)"));
        assert_eq!(
            winning(
                &sheets,
                ".focus-places-list > row:selected",
                "background-color"
            ),
            bar,
            "the folders popover leaves its highlighted row in the accent tint"
        );
    }

    #[test]
    fn a_result_and_the_thread_message_under_it_draw_the_same_ring() {
        let sheets = [SHARED, COLOURS, SURFACES];
        for selector in [
            ".focus-results-row.focus-results-cursor",
            ".focus-results-child.focus-results-cursor",
        ] {
            assert_eq!(
                winning(&sheets, selector, "outline").as_deref(),
                Some("2px solid var(--postio-accent)"),
                "{selector}"
            );
        }
    }

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
