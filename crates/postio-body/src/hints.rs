//! Presentational attributes, translated into the inline style they mean.
//!
//! Spec 006 FR-007, research R9. Mail is still written with the attributes
//! HTML 4 gave it -- `<font color>`, `valign`, `cellpadding`, `<table
//! border>`, `align=center` -- because that is what renders in the mail
//! clients senders test against. Browsers honour every one of them through
//! presentational hints; a renderer that does not honour them collapses the
//! layout. So they are rewritten here, into the declarations a browser would
//! derive from them, where it is a pure function of markup and provable in
//! milliseconds -- and where every frontend's renderer inherits it.
//!
//! What this writes goes into the element's `style` *before* the sender's own
//! inline style, so where a sender wrote both, their declaration wins, as it
//! would in a browser. Everything written here then passes through the
//! sanitizer's refusals like anything a sender writes.

use html5ever::tendril::StrTendril;
use html5ever::{Attribute, LocalName, QualName, ns};
use markup5ever_rcdom::{Handle, NodeData};

use crate::sanitize::{is_colour, is_remote};

/// The largest length, in pixels, a numeric presentational attribute is
/// taken at. A `cellpadding="99999"` is not a layout anyone meant.
const MAX_PX: u32 = 200;

/// The grey browsers draw a `<table border>` in.
const BORDER_GREY: &str = "#808080";

/// What a `<table>` says about the cells directly inside it.
#[derive(Clone, Copy, Default)]
struct Table {
    padding: Option<u32>,
    bordered: bool,
}

/// Rewrite the presentational attributes under `node` into inline style.
///
/// Returns whether anything changed, so the caller serializes the document
/// again only when there is something to serialize.
pub(crate) fn apply(node: &Handle) -> bool {
    let mut changed = false;
    walk(node, None, &mut changed);
    changed
}

fn walk(node: &Handle, table: Option<Table>, changed: &mut bool) {
    let mut inner = table;
    if let NodeData::Element { name, attrs, .. } = &node.data {
        let element = name.local.as_ref().to_ascii_lowercase();
        let get = |wanted: &str| {
            attrs
                .borrow()
                .iter()
                .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(wanted))
                .map(|attr| attr.value.trim().to_owned())
        };
        let mut hint: Vec<String> = Vec::new();
        match element.as_str() {
            "font" => {
                if let Some(color) = get("color").filter(|c| is_colour(c)) {
                    hint.push(format!("color: {color}"));
                }
                if let Some(face) = get("face").filter(|f| is_font_list(f)) {
                    hint.push(format!("font-family: {face}"));
                }
                if let Some(size) = get("size").as_deref().and_then(legacy_font_size) {
                    hint.push(format!("font-size: {size}"));
                }
            }
            "table" => {
                if let Some(spacing) = get("cellspacing").as_deref().and_then(pixels) {
                    hint.push(format!("border-spacing: {spacing}px"));
                }
                let border = get("border").as_deref().and_then(pixels).unwrap_or(0);
                if border > 0 {
                    hint.push(format!("border: {border}px solid {BORDER_GREY}"));
                }
                match get("align").map(|a| a.to_ascii_lowercase()).as_deref() {
                    Some("center") => hint.push("margin-left: auto; margin-right: auto".to_owned()),
                    Some(side @ ("left" | "right")) => hint.push(format!("float: {side}")),
                    _ => {}
                }
                // A nested table starts over: its cells answer to it, not to
                // the table around it.
                inner = Some(Table {
                    padding: get("cellpadding").as_deref().and_then(pixels),
                    bordered: border > 0,
                });
            }
            "td" | "th" => {
                if let Some(table) = table {
                    if let Some(padding) = table.padding {
                        hint.push(format!("padding: {padding}px"));
                    }
                    if table.bordered {
                        hint.push(format!("border: 1px solid {BORDER_GREY}"));
                    }
                }
            }
            "img" => match get("align").map(|a| a.to_ascii_lowercase()).as_deref() {
                Some(side @ ("left" | "right")) => hint.push(format!("float: {side}")),
                Some(position @ ("top" | "middle" | "bottom")) => {
                    hint.push(format!("vertical-align: {position}"));
                }
                _ => {}
            },
            _ => {}
        }
        if matches!(
            element.as_str(),
            "td" | "th" | "tr" | "thead" | "tbody" | "tfoot" | "col" | "colgroup"
        ) && let Some(valign) = get("valign").map(|v| v.to_ascii_lowercase())
            && matches!(valign.as_str(), "top" | "middle" | "bottom" | "baseline")
        {
            hint.push(format!("vertical-align: {valign}"));
        }
        if matches!(element.as_str(), "table" | "td" | "th")
            && let Some(background) = get("background")
        {
            // Written as the sender would have written it in CSS; the
            // declaration then takes the same road any sender `url()` does,
            // which rewrites a `cid:` to this message's part and holds a
            // remote one back unless allowed.
            if plain_url(&background) && (background.starts_with("cid:") || is_remote(&background))
            {
                hint.push(format!("background-image: url({background})"));
            }
        }
        if !hint.is_empty() {
            prepend_style(&mut attrs.borrow_mut(), &hint.join("; "));
            *changed = true;
        }
    }
    for child in node.children.borrow().iter() {
        walk(child, inner, changed);
    }
}

/// Put `hint` in front of the element's own style, so the sender's own
/// declarations come later and win.
fn prepend_style(attrs: &mut Vec<Attribute>, hint: &str) {
    if let Some(style) = attrs
        .iter_mut()
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case("style"))
    {
        let own = style.value.trim().to_owned();
        style.value = StrTendril::from(if own.is_empty() {
            hint.to_owned()
        } else {
            format!("{hint}; {own}")
        });
        return;
    }
    attrs.push(Attribute {
        name: QualName::new(None, ns!(), LocalName::from("style")),
        value: StrTendril::from(hint),
    });
}

/// A URL that can sit in an unquoted `url()` without ending it early.
fn plain_url(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| !matches!(c, '(' | ')' | '"' | '\'' | '\\') && !c.is_whitespace())
}

/// A non-negative pixel count, as HTML's numeric attributes spell one.
fn pixels(value: &str) -> Option<u32> {
    let digits = value.trim().trim_end_matches("px");
    let number: u32 = digits.parse().ok()?;
    Some(number.min(MAX_PX))
}

/// A font list and nothing else: names, spaces, commas, hyphens and quotes.
fn is_font_list(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | ',' | '-' | '_' | '\'' | '"'))
}

/// HTML's legacy `<font size>` scale, as the CSS keywords it maps to.
///
/// 1 to 7, 3 the default; `+n` and `-n` count from 3; out-of-range values
/// clamp, as browsers do.
fn legacy_font_size(value: &str) -> Option<&'static str> {
    const SCALE: [&str; 7] = [
        "x-small",
        "small",
        "medium",
        "large",
        "x-large",
        "xx-large",
        "xxx-large",
    ];
    let value = value.trim();
    let size: i32 = if let Some(delta) = value.strip_prefix('+') {
        3 + delta.parse::<i32>().ok()?
    } else if let Some(delta) = value.strip_prefix('-') {
        3 - delta.parse::<i32>().ok()?
    } else {
        value.parse().ok()?
    };
    Some(SCALE[(size.clamp(1, 7) - 1) as usize])
}

#[cfg(test)]
mod tests {
    use crate::sanitize::{RemoteImages, sanitize_body_in};

    fn clean(html: &str) -> String {
        sanitize_body_in(html, RemoteImages::Blocked, Some("7")).html
    }

    #[test]
    fn font_becomes_the_style_it_meant() {
        let html =
            clean(r##"<font color="#1d5c1d" face="Times New Roman, serif" size="5">Show</font>"##);
        assert!(html.contains("color: #1d5c1d"), "{html}");
        assert!(
            html.contains("font-family: Times New Roman, serif"),
            "{html}"
        );
        assert!(html.contains("font-size: x-large"), "{html}");
        assert!(html.contains("Show"), "{html}");
    }

    /// HTML's legacy scale: 1 is x-small, 3 the default, 7 xxx-large; `+n`
    /// and `-n` count from 3.
    #[test]
    fn font_size_follows_the_legacy_scale() {
        for (size, expected) in [
            ("1", "x-small"),
            ("2", "small"),
            ("3", "medium"),
            ("4", "large"),
            ("6", "xx-large"),
            ("7", "xxx-large"),
            ("9", "xxx-large"),
            ("+1", "large"),
            ("-2", "x-small"),
        ] {
            let html = clean(&format!(r#"<font size="{size}">x</font>"#));
            assert!(
                html.contains(&format!("font-size: {expected}")),
                "{size}: {html}"
            );
        }
    }

    #[test]
    fn a_font_attribute_that_is_not_what_it_claims_is_dropped() {
        let html = clean(r#"<font color="red;position:fixed" face="x}{y" size="big">t</font>"#);
        assert!(!html.contains("position"), "{html}");
        assert!(!html.contains("font-family"), "{html}");
        assert!(!html.contains("font-size"), "{html}");
    }

    #[test]
    fn valign_becomes_vertical_align() {
        let html = clean(
            r#"<table><tr><td valign="bottom">a</td><td valign="MIDDLE">b</td></tr></table>"#,
        );
        assert!(html.contains("vertical-align: bottom"), "{html}");
        assert!(html.contains("vertical-align: middle"), "{html}");
    }

    #[test]
    fn cellpadding_pads_this_tables_cells_and_not_a_nested_tables() {
        let html = clean(concat!(
            r#"<table cellpadding="10"><tr><td>outer"#,
            r#"<table><tr><td>inner</td></tr></table>"#,
            r#"</td></tr></table>"#,
        ));
        assert_eq!(html.matches("padding: 10px").count(), 1, "{html}");
    }

    #[test]
    fn cellspacing_becomes_border_spacing() {
        let html = clean(r#"<table cellspacing="4"><tr><td>a</td></tr></table>"#);
        assert!(html.contains("border-spacing: 4px"), "{html}");
    }

    #[test]
    fn a_table_border_draws_on_the_table_and_its_cells() {
        let html = clean(r#"<table border="2"><tr><td>a</td><td>b</td></tr></table>"#);
        assert!(html.contains("border: 2px solid"), "{html}");
        assert_eq!(html.matches("border: 1px solid").count(), 2, "{html}");
        let none = clean(r#"<table border="0"><tr><td>a</td></tr></table>"#);
        assert!(!none.contains("solid"), "{none}");
    }

    #[test]
    fn a_centred_table_is_centred_by_its_margins() {
        let html = clean(r#"<table align="center"><tr><td>a</td></tr></table>"#);
        assert!(
            html.contains("margin-left: auto; margin-right: auto"),
            "{html}"
        );
    }

    #[test]
    fn an_aligned_image_floats() {
        let html = clean(r#"<img src="cid:a@example.com" align="right" alt="">"#);
        assert!(html.contains("float: right"), "{html}");
    }

    #[test]
    fn a_cell_background_resolves_through_the_cid_rewriting() {
        let html =
            clean(r#"<table><tr><td background="cid:stripe@example.com">a</td></tr></table>"#);
        assert!(
            html.contains("background-image: url(postio-cid:7/stripe%40example.com)"),
            "{html}"
        );
    }

    #[test]
    fn a_remote_cell_background_is_held_back_until_allowed() {
        let raw =
            r#"<table><tr><td background="https://beacon.example.com/bg.png">a</td></tr></table>"#;
        let blocked = sanitize_body_in(raw, RemoteImages::Blocked, Some("7"));
        assert!(!blocked.html.contains("beacon"), "{}", blocked.html);
        assert_eq!(blocked.remote_blocked, 1);
        let allowed = sanitize_body_in(raw, RemoteImages::Allowed, Some("7"));
        assert!(
            allowed
                .html
                .contains("background-image: url(https://beacon.example.com/bg.png)"),
            "{}",
            allowed.html
        );
    }

    /// The sender's own inline style comes after the hint, so it wins.
    #[test]
    fn a_senders_own_style_outranks_the_hint() {
        let html = clean(
            r#"<table><tr><td valign="top" style="vertical-align: bottom">a</td></tr></table>"#,
        );
        let hint = html.find("vertical-align: top").expect("the hint");
        let own = html
            .find("vertical-align: bottom")
            .expect("the sender's own");
        assert!(hint < own, "{html}");
    }
}
