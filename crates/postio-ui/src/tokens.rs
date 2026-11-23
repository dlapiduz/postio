//! The design system's tokens, parsed and emitted for each frontend. `std` only: the
//! widgets `build.rs` compiles this file directly, so build and tests run the same generator.

use std::collections::BTreeMap;
use std::fmt::Write as _;

const REQUIRED: &[&str] = &[
    "color-bg",
    "color-surface",
    "color-text",
    "color-accent",
    "color-divider",
    "font-heading",
    "font-heading-weight",
    "font-body",
    "radius-sm",
    "radius-md",
    "radius-lg",
    "space-1",
    "space-2",
    "space-3",
    "space-4",
    "space-6",
    "space-8",
    "shadow-sm",
    "shadow-md",
    "shadow-lg",
];

/// Checked up front so a retuned design system fails the build loudly.
const REQUIRED_RAMPS: &[&str] = &[
    "color-neutral-100",
    "color-neutral-200",
    "color-neutral-300",
    "color-neutral-400",
    "color-neutral-500",
    "color-neutral-600",
    "color-neutral-700",
    "color-neutral-800",
    "color-neutral-900",
    "color-accent-100",
    "color-accent-200",
    "color-accent-300",
    "color-accent-400",
    "color-accent-500",
    "color-accent-600",
    "color-accent-700",
    "color-accent-800",
    "color-accent-900",
];

/// A chip is sized by its content plus these, never by a width.
const CHIP_HEIGHT: &str = "22px";
const CHIP_PAD_X: &str = "7px";
const CHIP_PAD_Y: &str = "2px";

#[derive(Debug)]
pub struct TokenError(pub String);

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TokenError {}

fn err<T>(msg: impl Into<String>) -> Result<T, TokenError> {
    Err(TokenError(msg.into()))
}

#[derive(Debug, Clone, Default)]
pub struct Tokens {
    order: Vec<String>,
    values: BTreeMap<String, String>,
}

impl Tokens {
    pub fn parse(css: &str) -> Result<Self, TokenError> {
        let css = strip_comments(css);
        let start = match css.find(":root") {
            Some(i) => i,
            None => return err("no `:root` block in the source stylesheet"),
        };
        let open = match css[start..].find('{') {
            Some(i) => start + i + 1,
            None => return err("`:root` is not followed by a block"),
        };
        let mut depth = 1usize;
        let mut end = None;
        for (i, c) in css[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = match end {
            Some(e) => e,
            None => return err("unterminated `:root` block"),
        };

        let mut tokens = Tokens::default();
        for decl in split_declarations(&css[open..end]) {
            let (name, value) = match decl.split_once(':') {
                Some(pair) => pair,
                None => continue,
            };
            let name = name.trim();
            let Some(name) = name.strip_prefix("--") else {
                continue;
            };
            let value = normalise_value(value.trim())?;
            if tokens.values.insert(name.to_string(), value).is_none() {
                tokens.order.push(name.to_string());
            }
        }

        for required in REQUIRED.iter().chain(REQUIRED_RAMPS) {
            if !tokens.values.contains_key(*required) {
                return err(format!(
                    "the design system no longer defines `--{required}`; \
                     update crates/postio-ui/src/tokens.rs to match"
                ));
            }
        }
        Ok(tokens)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn need(&self, name: &str) -> Result<&str, TokenError> {
        match self.get(name) {
            Some(v) => Ok(v),
            None => err(format!("missing design token `--{name}`")),
        }
    }

    /// For tests: proves retuning the source changes the generated CSS. `build.rs` never calls it.
    #[allow(dead_code)]
    pub fn set(&mut self, name: &str, value: &str) {
        if !self.values.contains_key(name) {
            self.order.push(name.to_string());
        }
        self.values.insert(name.to_string(), value.to_string());
    }

    fn tint(&self, name: &str, percent: f32) -> Result<String, TokenError> {
        let rgb = parse_hex(self.need(name)?).ok_or_else(|| {
            TokenError(format!(
                "`--{name}` is not a plain hex colour; cannot tint it"
            ))
        })?;
        Ok(rgba(rgb, percent / 100.0))
    }
}

/// In `rem` against GTK's 11pt default, so text scaling moves them all. Named here so a stylesheet
/// says `var(--postio-text-body)` rather than retyping a value.
pub const TYPE_ROLES: &[(&str, &str)] = &[
    ("micro", "0.6818rem"),
    ("meta", "0.7159rem"),
    ("small", "0.7841rem"),
    ("label", "0.8182rem"),
    ("ui", "0.8523rem"),
    ("body", "0.8864rem"),
    ("reading", "0.9204rem"),
    ("title", "1.3636rem"),
];

/// Colours become `NSColor`, lengths `CGFloat`, font families `String`. No dark variant is emitted
/// yet.
pub fn generate_swift(tokens: &Tokens, source: &str) -> Result<String, TokenError> {
    let mut out = String::with_capacity(8 * 1024);
    writeln!(out, "// GENERATED FILE — do not edit by hand.").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// Source     : {source}").unwrap();
    writeln!(out, "// Emitted by : crates/postio-ui/src/tokens.rs").unwrap();
    writeln!(out, "// Regenerate : scripts/macos-build.sh").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(
        out,
        "// Retune the design system's :root block and every value below follows —"
    )
    .unwrap();
    writeln!(
        out,
        "// on both frontends at once, which is the point of it being generated."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "import AppKit").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "/// Postio's design tokens.").unwrap();
    writeln!(out, "public enum PostioTokens {{").unwrap();

    for name in tokens.names() {
        let Some(value) = tokens.get(name) else {
            continue;
        };
        let swift = swift_name(name);
        if let Some(components) = colour(value) {
            let (r, g, b, a) = components;
            writeln!(
                out,
                "    /// `--postio-{name}`: `{value}`\n    public static let {swift} = NSColor(srgbRed: {r:.4}, green: {g:.4}, blue: {b:.4}, alpha: {a:.4})"
            )
            .unwrap();
        } else if let Some(points) = length(value) {
            writeln!(
                out,
                "    /// `--postio-{name}`: `{value}`\n    public static let {swift}: CGFloat = {points}"
            )
            .unwrap();
        } else if name.starts_with("font-") && !name.ends_with("-weight") {
            let family = value
                .split(',')
                .next()
                .unwrap_or(value)
                .trim()
                .trim_matches('"');
            writeln!(
                out,
                "    /// `--postio-{name}`: `{value}`\n    public static let {swift} = \"{family}\""
            )
            .unwrap();
        }
        // No AppKit equivalent for shadows, weights or gradients; not guessed at.
    }

    // Row states are derived, not declared in `:root`, so the loop above never sees them. Emitted
    // as dynamic colours because values differ per theme.
    for (swift, doc, light, dark) in [
        (
            "colorSelectedBg",
            "the selected row's tint",
            tokens.tint("color-accent", 12.0)?,
            // The literal, not `var(--…)`: `colour` parses values and a CSS reference is not one.
            tokens
                .get("color-accent-900")
                .unwrap_or_default()
                .to_owned(),
        ),
        (
            "colorSelectedStrongBg",
            "the selected row's tint, one step stronger",
            tokens.tint("color-accent", 14.0)?,
            tokens
                .get("color-accent-900")
                .unwrap_or_default()
                .to_owned(),
        ),
        (
            "colorHoverBg",
            "the row under the pointer",
            tokens.tint("color-text", 4.0)?,
            tokens.tint("color-neutral-100", 6.0)?,
        ),
    ] {
        let Some(light) = colour(&light) else {
            continue;
        };
        let Some(dark) = colour(&dark) else { continue };
        writeln!(
            out,
            "    /// {doc} — light and dark, from the design system's two blocks."
        )
        .unwrap();
        writeln!(
            out,
            "    public static let {swift} = NSColor(name: nil) {{ appearance in\n             \x20       appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua\n             \x20           ? NSColor(srgbRed: {:.4}, green: {:.4}, blue: {:.4}, alpha: {:.4})\n             \x20           : NSColor(srgbRed: {:.4}, green: {:.4}, blue: {:.4}, alpha: {:.4})\n             \x20   }}",
            dark.0, dark.1, dark.2, dark.3, light.0, light.1, light.2, light.3
        )
        .unwrap();
    }

    writeln!(out, "}}").unwrap();
    Ok(out)
}

fn swift_name(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    let mut upper = false;
    for ch in token.chars() {
        if ch == '-' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

fn colour(value: &str) -> Option<(f32, f32, f32, f32)> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() != 6 {
            return None;
        }
        let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return Some((
            channel(0)? as f32 / 255.0,
            channel(2)? as f32 / 255.0,
            channel(4)? as f32 / 255.0,
            1.0,
        ));
    }
    let inner = value.strip_prefix("rgba(")?.strip_suffix(')')?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return None;
    }
    Some((
        parts[0].parse::<f32>().ok()? / 255.0,
        parts[1].parse::<f32>().ok()? / 255.0,
        parts[2].parse::<f32>().ok()? / 255.0,
        parts[3].parse::<f32>().ok()?,
    ))
}

/// Fractional on purpose: the spacing ramp is `3.4px` steps, and parsing to `u32` once silently
/// dropped every space token.
fn length(value: &str) -> Option<f32> {
    value.trim().strip_suffix("px")?.trim().parse().ok()
}

/// Rounded once here so a Rust margin and a stylesheet padding are the same number (the reader
/// keeps the fractional ramp).
fn whole_pixels(name: &str, value: &str) -> Option<i32> {
    if !name.starts_with("space-") {
        return None;
    }
    Some(length(value)?.round() as i32)
}

fn is_metric(name: &str) -> bool {
    name.starts_with("space-") || name.starts_with("radius-")
}

/// Colours are each app's own, so they are not emitted.
pub fn generate_metrics(tokens: &Tokens, source: &str) -> Result<String, TokenError> {
    let metrics: Vec<(&str, String)> = tokens
        .names()
        .filter(|name| is_metric(name))
        .map(|name| {
            let value = tokens.get(name).unwrap_or_default();
            let value = match whole_pixels(name, value) {
                Some(px) => format!("{px}px"),
                None => value.to_owned(),
            };
            (name, value)
        })
        .collect();
    for family in ["space-", "radius-"] {
        if !metrics.iter().any(|(name, _)| name.starts_with(family)) {
            return Err(TokenError(format!(
                "the design system has no `{family}*` tokens to generate"
            )));
        }
    }
    let mut out = String::with_capacity(2 * 1024);
    writeln!(out, "/* GENERATED FILE — do not edit by hand.").unwrap();
    writeln!(out, " *").unwrap();
    writeln!(out, " * Source : {source}").unwrap();
    writeln!(
        out,
        " * Emitted by: crates/postio-widgets/build.rs via postio_ui::tokens"
    )
    .unwrap();
    writeln!(out, " * Regenerate: cargo build -p postio-widgets").unwrap();
    writeln!(out, " *").unwrap();
    writeln!(
        out,
        " * The metrics the desktop app lays the shared widgets out by: spacing,"
    )
    .unwrap();
    writeln!(
        out,
        " * radii, chip sizes and type sizes (specs/007-postio-focus research R11)."
    )
    .unwrap();
    writeln!(
        out,
        " * No colour is defined here: the app defines its own, from libadwaita's"
    )
    .unwrap();
    writeln!(out, " * named colours.").unwrap();
    writeln!(out, " */\n").unwrap();
    writeln!(out, ":root {{").unwrap();
    for (name, value) in &metrics {
        writeln!(out, "  --postio-{name}: {value};").unwrap();
    }
    writeln!(
        out,
        "\n  /* A text chip's fixed vertical metrics: Postio's own. */"
    )
    .unwrap();
    writeln!(out, "  --postio-chip-height: {CHIP_HEIGHT};").unwrap();
    writeln!(out, "  --postio-chip-pad-x: {CHIP_PAD_X};").unwrap();
    writeln!(out, "  --postio-chip-pad-y: {CHIP_PAD_Y};").unwrap();
    writeln!(
        out,
        "\n  /* The sizes Postio sets type at, in `rem` so text scaling moves them. */"
    )
    .unwrap();
    for (role, size) in TYPE_ROLES {
        writeln!(out, "  --postio-text-{role}: {size};").unwrap();
    }
    writeln!(out, "}}").unwrap();
    Ok(out)
}

pub fn space_scale(tokens: &Tokens) -> Vec<(u32, i32)> {
    tokens
        .names()
        .filter_map(|name| {
            let step = name.strip_prefix("space-")?.parse().ok()?;
            Some((step, whole_pixels(name, tokens.get(name)?)?))
        })
        .collect()
}

pub fn generate_space_rs(tokens: &Tokens, source: &str) -> Result<String, TokenError> {
    let scale = space_scale(tokens);
    if scale.is_empty() {
        return Err(TokenError(
            "the design system has no `space-*` tokens to generate".to_owned(),
        ));
    }
    let mut out = String::new();
    writeln!(out, "// GENERATED FILE — do not edit by hand.").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// Source     : {source}").unwrap();
    writeln!(
        out,
        "// Emitted by : crates/postio-widgets/build.rs via postio_ui::tokens"
    )
    .unwrap();
    writeln!(out, "// Regenerate : cargo build -p postio-widgets").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(
        out,
        "// The design system's spacing ramp in whole pixels, the same numbers"
    )
    .unwrap();
    writeln!(out, "// `--postio-space-N` carries in metrics.css.").unwrap();
    writeln!(out).unwrap();
    for (step, px) in scale {
        writeln!(out, "/// `--postio-space-{step}`: {px}px.").unwrap();
        writeln!(out, "pub const S{step}: i32 = {px};").unwrap();
    }
    Ok(out)
}

/// Emits literal values: a `WebView` has no GTK style context for `--postio-*` variables. WebKit
/// honours `prefers-color-scheme` itself, so no dark class is needed.
pub fn generate_reader(tokens: &Tokens, source: &str) -> Result<String, TokenError> {
    let mut out = String::with_capacity(2 * 1024);

    writeln!(out, "/* GENERATED FILE — do not edit by hand.").unwrap();
    writeln!(out, " *").unwrap();
    writeln!(out, " * Source : {source}").unwrap();
    writeln!(
        out,
        " * Emitted by: crates/postio-widgets/build.rs via postio_ui::tokens"
    )
    .unwrap();
    writeln!(out, " * Regenerate: cargo build -p postio-widgets").unwrap();
    writeln!(out, " *").unwrap();
    writeln!(
        out,
        " * The `--r-*` custom properties data/reader.css's structural rules\n\
         \x20* reference. A WebView has its own CSS engine with no notion of the GTK\n\
         \x20* style context tokens.css's `--postio-*` variables live on, so these are\n\
         \x20* literal values computed from the same Tokens — same parser, same drift\n\
         \x20* test (tests/reader_tokens.rs), a different role mapping for the pane."
    )
    .unwrap();
    writeln!(out, " */\n").unwrap();

    write_scheme(&mut out, ":root", &reader_light_roles(tokens)?)?;

    writeln!(out, "@media (prefers-color-scheme: dark) {{").unwrap();
    writeln!(out, "  :root {{").unwrap();
    for (name, value) in reader_dark_roles(tokens)? {
        writeln!(out, "    {name}: {value};").unwrap();
    }
    writeln!(out, "  }}").unwrap();
    writeln!(out, "}}").unwrap();

    Ok(out)
}

fn reader_light_roles(t: &Tokens) -> Result<Vec<(&'static str, String)>, TokenError> {
    Ok(vec![
        ("--r-ground", t.need("color-neutral-100")?.to_string()),
        ("--r-ink", t.need("color-text")?.to_string()),
        ("--r-ink-secondary", t.tint("color-text", 80.0)?),
        ("--r-dim", t.tint("color-text", 55.0)?),
        ("--r-hairline", t.need("color-divider")?.to_string()),
        (
            "--r-hairline-strong",
            t.need("color-neutral-400")?.to_string(),
        ),
        ("--r-radius", t.need("radius-sm")?.to_string()),
        ("--r-accent", t.need("color-accent")?.to_string()),
        ("--r-quote-bg", t.tint("color-accent", 6.0)?),
        ("--r-match-bg", t.tint("color-accent", 28.0)?),
    ])
}

fn reader_dark_roles(t: &Tokens) -> Result<Vec<(&'static str, String)>, TokenError> {
    Ok(vec![
        (
            "--r-ground",
            t.need("color-reader-dark-ground")?.to_string(),
        ),
        ("--r-ink", t.need("color-reader-dark-ink")?.to_string()),
        (
            "--r-ink-secondary",
            t.need("color-reader-dark-ink-secondary")?.to_string(),
        ),
        ("--r-dim", t.need("color-reader-dark-dim")?.to_string()),
        (
            "--r-hairline",
            t.need("color-reader-dark-hairline")?.to_string(),
        ),
        (
            "--r-hairline-strong",
            t.need("color-reader-dark-hairline-strong")?.to_string(),
        ),
        (
            "--r-accent",
            t.need("color-reader-dark-accent")?.to_string(),
        ),
        ("--r-quote-bg", t.tint("color-reader-dark-accent", 8.0)?),
        ("--r-match-bg", t.tint("color-reader-dark-accent", 32.0)?),
    ])
}

fn write_scheme(
    out: &mut String,
    selector: &str,
    decls: &[(&'static str, String)],
) -> Result<(), TokenError> {
    writeln!(out, "{selector} {{").unwrap();
    for (name, value) in decls {
        if name.is_empty() {
            writeln!(out).unwrap();
            writeln!(out, "  /* {value} */").unwrap();
        } else {
            writeln!(out, "  {name}: {value};").unwrap();
        }
    }
    writeln!(out, "}}\n").unwrap();
    Ok(())
}

/// Account n gets hue `n % ACCOUNT_HUES`, so an account's colour is stable while its position is.
pub const ACCOUNT_HUES: usize = 8;

fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            match css[i + 2..].find("*/") {
                Some(end) => i = i + 2 + end + 2,
                None => break,
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn split_declarations(block: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in block.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ';' if depth == 0 => {
                if !current.trim().is_empty() {
                    out.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn normalise_value(value: &str) -> Result<String, TokenError> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let folded = fold_color_mix(&collapsed)?;
    Ok(drop_web_font_families(&folded))
}

/// Folded here so the generated sheet is free of version-dependent CSS.
fn fold_color_mix(value: &str) -> Result<String, TokenError> {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("color-mix(") {
        out.push_str(&rest[..start]);
        let after = &rest[start + "color-mix(".len()..];
        let Some(close) = matching_paren(after) else {
            return err(format!("unterminated color-mix() in `{value}`"));
        };
        let args = &after[..close];
        out.push_str(&fold_one_color_mix(args, value)?);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn fold_one_color_mix(args: &str, whole: &str) -> Result<String, TokenError> {
    let parts: Vec<&str> = args.split(',').map(str::trim).collect();
    if parts.len() != 3 || parts[0] != "in srgb" {
        return err(format!(
            "only `color-mix(in srgb, <hex> N%, transparent)` is understood, got `{whole}`"
        ));
    }
    if parts[2] != "transparent" {
        return err(format!(
            "only a mix towards `transparent` is understood, got `{whole}`"
        ));
    }
    let (color, percent) = match parts[1].rsplit_once(' ') {
        Some(pair) => pair,
        None => return err(format!("no percentage in `{whole}`")),
    };
    let percent: f32 = match percent.trim_end_matches('%').parse() {
        Ok(p) => p,
        Err(_) => return err(format!("unreadable percentage in `{whole}`")),
    };
    let Some(rgb) = parse_hex(color) else {
        return err(format!("`{color}` is not a hex colour in `{whole}`"));
    };
    Ok(rgba(rgb, percent / 100.0))
}

fn matching_paren(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// `system-ui` is a web keyword; GTK would treat it as a family name.
fn drop_web_font_families(value: &str) -> String {
    if !value.contains("system-ui") {
        return value.to_string();
    }
    value
        .split(',')
        .map(str::trim)
        .filter(|f| *f != "system-ui")
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_hex(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.trim().strip_prefix('#')?;
    let expand = |c: u8| -> u8 { (c << 4) | c };
    match hex.len() {
        3 => {
            let d: Vec<u8> = hex.bytes().map(|b| hex_digit(b).unwrap_or(255)).collect();
            if d.contains(&255) {
                return None;
            }
            Some((expand(d[0]), expand(d[1]), expand(d[2])))
        }
        6 => {
            let mut v = [0u8; 3];
            for (i, channel) in v.iter_mut().enumerate() {
                let hi = hex_digit(hex.as_bytes()[i * 2])?;
                let lo = hex_digit(hex.as_bytes()[i * 2 + 1])?;
                *channel = (hi << 4) | lo;
            }
            Some((v[0], v[1], v[2]))
        }
        _ => None,
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn rgba((r, g, b): (u8, u8, u8), alpha: f32) -> String {
    let alpha = (alpha * 100.0).round() / 100.0;
    format!("rgba({r}, {g}, {b}, {alpha})")
}

/// Read from the generated reader palette, not retyped, so retuning the design system moves every
/// frontend.
pub fn accent_rgb() -> ((u8, u8, u8), (u8, u8, u8)) {
    const PALETTE: &str = include_str!("../data/reader-tokens.css");
    let mut accents = PALETTE.lines().filter_map(|line| {
        let value = line.trim().strip_prefix("--r-accent:")?.trim();
        let hex = value.strip_prefix('#')?.strip_suffix(';')?;
        let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        Some((channel(0)?, channel(2)?, channel(4)?))
    });
    let light = accents.next().unwrap_or_default();
    let dark = accents.next().unwrap_or(light);
    (light, dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        /* a comment */
        :root {
          --color-bg: #f2f2f3;
          --color-divider: color-mix(in srgb, #1d1f20 16%, transparent);
          --font-body: "Barlow", system-ui, sans-serif;
          --space-1: 3.4px;
        }
        body { background: var(--color-bg); }
    "#;

    #[test]
    fn reads_the_root_block_and_stops_at_its_brace() {
        let css = strip_comments(SAMPLE);
        assert!(!css.contains("a comment"));
        let block = &css[css.find(":root").unwrap()..];
        let decls =
            split_declarations(&block[block.find('{').unwrap() + 1..block.find('}').unwrap()]);
        assert_eq!(decls.len(), 4, "{decls:?}");
        assert!(decls.iter().all(|d| d.starts_with("--")));
    }

    #[test]
    fn folds_color_mix_to_rgba() {
        assert_eq!(
            normalise_value("color-mix(in srgb, #1d1f20 16%, transparent)").unwrap(),
            "rgba(29, 31, 32, 0.16)"
        );
    }

    #[test]
    fn drops_system_ui() {
        assert_eq!(
            normalise_value("\"Barlow\", system-ui, sans-serif").unwrap(),
            "\"Barlow\", sans-serif"
        );
    }

    #[test]
    fn folds_color_mix_inside_a_shadow() {
        assert_eq!(
            normalise_value("0 1px 2px color-mix(in srgb, #2b2b2d 14%, transparent)").unwrap(),
            "0 1px 2px rgba(43, 43, 45, 0.14)"
        );
    }

    #[test]
    fn expands_three_digit_hex() {
        assert_eq!(parse_hex("#abc"), Some((0xaa, 0xbb, 0xcc)));
    }

    #[test]
    fn the_accent_is_read_from_the_generated_palette_for_both_schemes() {
        let (light, dark) = super::accent_rgb();
        let hex = |(r, g, b): (u8, u8, u8)| format!("#{r:02x}{g:02x}{b:02x}");
        let palette = include_str!("../data/reader-tokens.css");
        assert!(
            palette.contains(&format!("--r-accent: {};", hex(light))),
            "{light:?}"
        );
        assert!(
            palette.contains(&format!("--r-accent: {};", hex(dark))),
            "{dark:?}"
        );
        assert_ne!(light, dark, "dark mode has an accent of its own");
    }
}

#[cfg(test)]
mod token_error_tests {
    use super::{TokenError, err};

    #[test]
    fn a_token_error_displays_its_message() {
        let failed: Result<(), TokenError> = err("no --color-accent in :root");
        let error = failed.expect_err("err builds an error");
        assert_eq!(error.to_string(), "no --color-accent in :root");
        let boxed: Box<dyn std::error::Error> = Box::new(error);
        assert_eq!(boxed.to_string(), "no --color-accent in :root");
    }
}
