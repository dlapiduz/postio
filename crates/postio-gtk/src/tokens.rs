//! The Industry design system, read at build time and emitted as GTK CSS.
//!
//! This module is deliberately dependency-free (`std` only) because `build.rs`
//! compiles it directly (`#[path = "src/tokens.rs"] mod tokens;`): the build
//! script and the test suite run *exactly* the same parser and generator, so
//! `data/tokens.css` can be checked for drift by a test rather than by eye.
//!
//! The pipeline is:
//!
//! ```text
//! Design/_ds/industry-*/styles.css   :root { --color-*, --font-*, --space-*, … }
//!            |  parse()
//!            v
//!        Tokens                      name -> value, source order preserved
//!            |  generate()
//!            v
//! crates/postio-gtk/data/tokens.css  :root { … }  :root.postio-dark { … }  …
//! ```
//!
//! Nothing here retypes a value from the design system: every colour, length,
//! radius and font stack in the output is either copied from the parsed token
//! or computed from one (an alpha tint, a ramp step). Retune the source
//! `styles.css` and the app follows.
//!
//! ## Why classes and not `@media (prefers-color-scheme: dark)`
//!
//! GTK does support that media query, but only for the *theme* provider, which
//! it loads with an explicit `dark`/`hc` variant. In an application-priority
//! provider the query never matches (verified on GTK 4.22.4). libadwaita does
//! not tag the widget tree either. So the scheme-dependent blocks below are
//! keyed off `:root.postio-dark` / `:root.postio-hc`, and `crate::style` keeps
//! those classes in sync with `AdwStyleManager`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Every token the generator needs from the source `:root` block. Anything the
/// design system adds beyond this list is still carried through verbatim.
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

/// Ramp steps the semantic layer maps onto. Checked up front so a retuned
/// design system fails the build loudly instead of emitting a broken sheet.
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

/// The mono face is Postio's own addition: the Industry system has no
/// monospace role, but the mail canvas puts counts, key hints and metadata in
/// IBM Plex Mono. Kept here so it travels with the other font tokens.
const FONT_MONO: &str = "\"IBM Plex Mono\", monospace";

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

/// The parsed `:root` block, in source order.
#[derive(Debug, Clone, Default)]
pub struct Tokens {
    order: Vec<String>,
    values: BTreeMap<String, String>,
}

impl Tokens {
    /// Parse the first `:root { … }` block of a design-system stylesheet.
    ///
    /// Values are normalised for GTK on the way in: comments dropped,
    /// whitespace collapsed, `color-mix(in srgb, <hex> N%, transparent)`
    /// folded to `rgba()`, and web-only font families (`system-ui`) removed.
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
                     update crates/postio-gtk/src/tokens.rs to match"
                ));
            }
        }
        Ok(tokens)
    }

    /// Token names in source order.
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

    /// Override a token — used by tests to prove that retuning the source
    /// stylesheet really does change the generated CSS. (`build.rs` includes
    /// this module and never calls it.)
    #[allow(dead_code)]
    pub fn set(&mut self, name: &str, value: &str) {
        if !self.values.contains_key(name) {
            self.order.push(name.to_string());
        }
        self.values.insert(name.to_string(), value.to_string());
    }

    /// `--postio-<name>`, the generated variable a raw token lands in.
    fn var(&self, name: &str) -> Result<String, TokenError> {
        self.need(name)?;
        Ok(format!("var(--postio-{name})"))
    }

    /// A token tinted to `percent` opacity, folded to a literal `rgba()`.
    fn tint(&self, name: &str, percent: f32) -> Result<String, TokenError> {
        let rgb = parse_hex(self.need(name)?).ok_or_else(|| {
            TokenError(format!(
                "`--{name}` is not a plain hex colour; cannot tint it"
            ))
        })?;
        Ok(rgba(rgb, percent / 100.0))
    }
}

/// Generate `data/tokens.css` from the parsed design system.
pub fn generate(tokens: &Tokens, source: &str) -> Result<String, TokenError> {
    let mut out = String::with_capacity(8 * 1024);

    writeln!(out, "/* GENERATED FILE — do not edit by hand.").unwrap();
    writeln!(out, " *").unwrap();
    writeln!(out, " * Source : {source}").unwrap();
    writeln!(
        out,
        " * Emitted by: crates/postio-gtk/build.rs via crates/postio-gtk/src/tokens.rs"
    )
    .unwrap();
    writeln!(out, " * Regenerate: cargo build -p postio-gtk").unwrap();
    writeln!(out, " *").unwrap();
    writeln!(
        out,
        " * Retune the design system's :root block and every value below follows."
    )
    .unwrap();
    writeln!(
        out,
        " * The Industry identity is kept — Barlow Condensed / Barlow / IBM Plex Mono,"
    )
    .unwrap();
    writeln!(
        out,
        " * the steel accent, hairline dividers, airy rows. Its wireframe chrome is not:"
    )
    .unwrap();
    writeln!(
        out,
        " * no blueprint corner registration marks, no transparent line-drawing cards."
    )
    .unwrap();
    writeln!(out, " */\n").unwrap();

    // ── 1. the design system's own tokens, carried through verbatim ────────
    writeln!(
        out,
        "/* ── Industry tokens ─────────────────────────────────────────────────\n\
         \x20  Straight from the source :root, one `--postio-` prefixed variable each.\n\
         \x20  Scheme-independent: these are the raw material, not the roles. */"
    )
    .unwrap();
    writeln!(out, ":root {{").unwrap();
    for name in tokens.names() {
        let value = tokens.get(name).unwrap_or_default();
        writeln!(out, "  --postio-{name}: {value};").unwrap();
    }
    writeln!(
        out,
        "\n  /* Postio's own: the Industry system has no monospace role, but the mail\n\
         \x20    canvas sets counts, key hints and metadata in IBM Plex Mono. */"
    )
    .unwrap();
    writeln!(out, "  --postio-font-mono: {FONT_MONO};").unwrap();
    writeln!(out, "}}\n").unwrap();

    // ── 2. semantic roles + Adwaita named colours, light ───────────────────
    writeln!(
        out,
        "/* ── Roles — light ───────────────────────────────────────────────────\n\
         \x20  Semantic names the widgets use, then the libadwaita named colours so\n\
         \x20  stock GTK widgets sit on the Industry ground instead of Adwaita's. */"
    )
    .unwrap();
    write_scheme(&mut out, ":root", &light_roles(tokens)?)?;

    // ── 3. dark ────────────────────────────────────────────────────────────
    writeln!(
        out,
        "/* ── Roles — dark ────────────────────────────────────────────────────\n\
         \x20  Canvas 3c: the board sits on the neutral deep step, the selected row on\n\
         \x20  the accent's deep step, steel goes light-on-dark (accent-400 fills,\n\
         \x20  accent-300 text) and the hairlines LIFT to neutral-700 rather than\n\
         \x20  darkening. `--accent-color` is left to libadwaita, which derives the\n\
         \x20  standalone step from `--accent-bg-color` per scheme. */"
    )
    .unwrap();
    write_scheme(&mut out, ":root.postio-dark", &dark_roles(tokens)?)?;

    // ── 4. high contrast ───────────────────────────────────────────────────
    writeln!(
        out,
        "/* ── Roles — high contrast ───────────────────────────────────────────\n\
         \x20  Only the things that carry meaning at low contrast move: hairlines\n\
         \x20  gain weight, dimmed text comes back up, accent text drops to a deeper\n\
         \x20  ramp step. The ground and the identity stay put. */"
    )
    .unwrap();
    write_scheme(&mut out, ":root.postio-hc", &light_hc_roles(tokens)?)?;
    write_scheme(
        &mut out,
        ":root.postio-dark.postio-hc",
        &dark_hc_roles(tokens)?,
    )?;

    // ── 5. type roles ──────────────────────────────────────────────────────
    writeln!(
        out,
        "/* ── Type roles ──────────────────────────────────────────────────────\n\
         \x20  Barlow Condensed headings over Barlow body, IBM Plex Mono for counts,\n\
         \x20  key hints and metadata. The faces ship in the GResource bundle and are\n\
         \x20  registered at startup (see `crate::fonts`), so none of this depends on\n\
         \x20  a system font installation. */"
    )
    .unwrap();
    writeln!(out, ":root {{").unwrap();
    writeln!(out, "  font-family: var(--postio-font-body);").unwrap();
    writeln!(out, "}}\n").unwrap();
    writeln!(
        out,
        ".postio-heading,\n\
         .postio-title,\n\
         .postio-kicker {{\n\
         \x20 font-family: var(--postio-font-heading);\n\
         \x20 font-weight: var(--postio-font-heading-weight);\n\
         }}\n"
    )
    .unwrap();
    writeln!(
        out,
        ".postio-kicker {{\n\
         \x20 font-size: 0.6818rem;\n\
         \x20 letter-spacing: 0.18em;\n\
         \x20 text-transform: uppercase;\n\
         \x20 color: var(--postio-faint);\n\
         }}\n"
    )
    .unwrap();
    writeln!(
        out,
        ".postio-mono,\n\
         .postio-count,\n\
         .postio-meta,\n\
         .postio-key {{\n\
         \x20 font-family: var(--postio-font-mono);\n\
         }}\n"
    )
    .unwrap();
    writeln!(
        out,
        ".postio-meta {{\n\
         \x20 color: var(--postio-dim);\n\
         }}\n"
    )
    .unwrap();
    writeln!(
        out,
        "/* A key hint: the mnemonic shown on the focused row, never a decoration. */\n\
         .postio-key {{\n\
         \x20 color: var(--postio-key-fg);\n\
         \x20 border: 1px solid var(--postio-key-border);\n\
         \x20 border-radius: var(--postio-radius-sm);\n\
         \x20 padding: 1px 4px;\n\
         }}\n"
    )
    .unwrap();
    writeln!(
        out,
        "/* A hairline divider — the Industry rule, one device-pixel of ink. */\n\
         .postio-hairline {{\n\
         \x20 background-color: var(--postio-hairline);\n\
         \x20 min-width: 1px;\n\
         \x20 min-height: 1px;\n\
         }}"
    )
    .unwrap();

    Ok(out)
}

/// Generate `data/reader-tokens.css` from the parsed design system: the
/// `--r-*` custom properties `data/reader.css`'s structural rules reference.
///
/// A `WebView` has its own CSS engine with no notion of the GTK style
/// context `--postio-*` variables live on (see [`generate`]'s module docs),
/// so this emits literal values — the same parser and the same tint/ramp
/// math, mapped onto the reader's own, smaller role set. Unlike GTK, WebKit
/// honours `@media (prefers-color-scheme: dark)` directly, so the reader
/// needs no `postio-dark` class equivalent.
pub fn generate_reader(tokens: &Tokens, source: &str) -> Result<String, TokenError> {
    let mut out = String::with_capacity(2 * 1024);

    writeln!(out, "/* GENERATED FILE — do not edit by hand.").unwrap();
    writeln!(out, " *").unwrap();
    writeln!(out, " * Source : {source}").unwrap();
    writeln!(
        out,
        " * Emitted by: crates/postio-gtk/build.rs via crates/postio-gtk/src/tokens.rs"
    )
    .unwrap();
    writeln!(out, " * Regenerate: cargo build -p postio-gtk").unwrap();
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
        // High-contrast weight for the body container's edge (#323) — same
        // step tokens.css's own `--postio-hairline-strong` uses in light.
        (
            "--r-hairline-strong",
            t.need("color-neutral-400")?.to_string(),
        ),
        // Scheme-independent, so defined once here rather than in both roles.
        ("--r-radius", t.need("radius-sm")?.to_string()),
        ("--r-accent", t.need("color-accent")?.to_string()),
        ("--r-quote-bg", t.tint("color-accent", 6.0)?),
        ("--r-match-bg", t.tint("color-accent", 28.0)?),
    ])
}

fn reader_dark_roles(t: &Tokens) -> Result<Vec<(&'static str, String)>, TokenError> {
    Ok(vec![
        ("--r-ground", t.need("color-neutral-900")?.to_string()),
        ("--r-ink", t.need("color-neutral-100")?.to_string()),
        (
            "--r-ink-secondary",
            t.need("color-neutral-200")?.to_string(),
        ),
        ("--r-dim", t.need("color-neutral-400")?.to_string()),
        ("--r-hairline", t.need("color-neutral-700")?.to_string()),
        (
            "--r-hairline-strong",
            t.need("color-neutral-600")?.to_string(),
        ),
        ("--r-accent", t.need("color-accent-400")?.to_string()),
        ("--r-quote-bg", t.tint("color-accent-400", 8.0)?),
        ("--r-match-bg", t.tint("color-accent-400", 32.0)?),
    ])
}

/// One scheme block: semantic roles first, then the libadwaita overrides they
/// feed. Both lists are ordered, so the output is byte-reproducible.
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

fn light_roles(t: &Tokens) -> Result<Vec<(&'static str, String)>, TokenError> {
    let ground = t.var("color-bg")?;
    let surface = t.var("color-surface")?;
    let ink = t.var("color-text")?;
    let hairline = t.var("color-divider")?;
    let accent = t.var("color-accent")?;

    Ok(vec![
        ("", "Ground and ink".into()),
        ("--postio-ground", ground.clone()),
        ("--postio-surface", surface.clone()),
        ("--postio-ink", ink.clone()),
        ("--postio-ink-secondary", t.tint("color-text", 80.0)?),
        ("--postio-dim", t.tint("color-text", 55.0)?),
        ("--postio-faint", t.tint("color-text", 45.0)?),
        ("", "Hairlines — the only edge this design draws".into()),
        ("--postio-hairline", hairline.clone()),
        ("--postio-hairline-strong", t.var("color-neutral-400")?),
        ("", "Steel".into()),
        ("--postio-accent", accent.clone()),
        ("--postio-accent-fg", ground.clone()),
        ("--postio-accent-hover", t.var("color-accent-600")?),
        ("--postio-accent-active", t.var("color-accent-700")?),
        ("--postio-accent-text", t.var("color-accent-700")?),
        (
            "",
            "Row states — airy rows, a 3px steel edge when selected".into(),
        ),
        ("--postio-selected-bg", t.tint("color-accent", 12.0)?),
        ("--postio-selected-strong-bg", t.tint("color-accent", 14.0)?),
        ("--postio-selected-border", accent.clone()),
        ("--postio-selected-fg", ink.clone()),
        ("--postio-selected-accent-text", t.var("color-accent-800")?),
        ("--postio-hover-bg", t.tint("color-text", 4.0)?),
        ("--postio-active-bg", t.tint("color-text", 8.0)?),
        ("", "Key hints".into()),
        ("--postio-key-fg", t.var("color-neutral-500")?),
        ("--postio-key-border", t.var("color-neutral-300")?),
        ("", "libadwaita named colours".into()),
        ("--window-bg-color", ground.clone()),
        ("--window-fg-color", ink.clone()),
        ("--view-bg-color", ground.clone()),
        ("--view-fg-color", ink.clone()),
        ("--headerbar-bg-color", ground.clone()),
        ("--headerbar-fg-color", ink.clone()),
        ("--headerbar-border-color", hairline.clone()),
        ("--headerbar-backdrop-color", surface.clone()),
        ("--headerbar-shade-color", hairline.clone()),
        ("--headerbar-darker-shade-color", hairline.clone()),
        ("--sidebar-bg-color", ground.clone()),
        ("--sidebar-fg-color", ink.clone()),
        ("--sidebar-backdrop-color", surface.clone()),
        ("--sidebar-border-color", hairline.clone()),
        ("--sidebar-shade-color", hairline.clone()),
        ("--secondary-sidebar-bg-color", surface.clone()),
        ("--secondary-sidebar-fg-color", ink.clone()),
        ("--secondary-sidebar-backdrop-color", surface.clone()),
        ("--secondary-sidebar-border-color", hairline.clone()),
        ("--secondary-sidebar-shade-color", hairline.clone()),
        ("--card-bg-color", surface.clone()),
        ("--card-fg-color", ink.clone()),
        ("--card-shade-color", hairline.clone()),
        ("--dialog-bg-color", surface.clone()),
        ("--dialog-fg-color", ink.clone()),
        ("--popover-bg-color", surface.clone()),
        ("--popover-fg-color", ink.clone()),
        ("--popover-shade-color", hairline.clone()),
        ("--thumbnail-bg-color", surface.clone()),
        ("--thumbnail-fg-color", ink.clone()),
        ("--overview-bg-color", surface.clone()),
        ("--overview-fg-color", ink.clone()),
        ("--shade-color", hairline.clone()),
        ("--scrollbar-outline-color", ground.clone()),
        ("--accent-bg-color", accent.clone()),
        ("--accent-fg-color", ground.clone()),
    ])
}

fn dark_roles(t: &Tokens) -> Result<Vec<(&'static str, String)>, TokenError> {
    let ground = t.var("color-neutral-900")?;
    let surface = t.var("color-neutral-800")?;
    let ink = t.var("color-neutral-100")?;
    let hairline = t.var("color-neutral-700")?;
    let accent = t.var("color-accent-400")?;

    Ok(vec![
        ("", "Ground and ink".into()),
        ("--postio-ground", ground.clone()),
        ("--postio-surface", surface.clone()),
        ("--postio-ink", ink.clone()),
        ("--postio-ink-secondary", t.var("color-neutral-200")?),
        ("--postio-dim", t.var("color-neutral-400")?),
        ("--postio-faint", t.var("color-neutral-500")?),
        ("", "Hairlines lift instead of darkening".into()),
        ("--postio-hairline", hairline.clone()),
        ("--postio-hairline-strong", t.var("color-neutral-600")?),
        ("", "Steel, light-on-dark".into()),
        ("--postio-accent", accent.clone()),
        ("--postio-accent-fg", ground.clone()),
        ("--postio-accent-hover", t.var("color-accent-300")?),
        ("--postio-accent-active", t.var("color-accent-500")?),
        ("--postio-accent-text", t.var("color-accent-300")?),
        (
            "",
            "Row states — the selected row takes the accent's deep step".into(),
        ),
        ("--postio-selected-bg", t.var("color-accent-900")?),
        ("--postio-selected-strong-bg", t.var("color-accent-900")?),
        ("--postio-selected-border", accent.clone()),
        ("--postio-selected-fg", ink.clone()),
        ("--postio-selected-accent-text", t.var("color-accent-300")?),
        ("--postio-hover-bg", t.tint("color-neutral-100", 6.0)?),
        ("--postio-active-bg", t.tint("color-neutral-100", 10.0)?),
        ("", "Key hints".into()),
        ("--postio-key-fg", t.var("color-neutral-400")?),
        ("--postio-key-border", t.var("color-neutral-600")?),
        (
            "",
            "Elevation on a dark ground is ambient darkness, not ink tint".into(),
        ),
        ("--postio-shadow-sm", dark_shadow(t, "shadow-sm")?),
        ("--postio-shadow-md", dark_shadow(t, "shadow-md")?),
        ("--postio-shadow-lg", dark_shadow(t, "shadow-lg")?),
        ("", "libadwaita named colours".into()),
        ("--window-bg-color", ground.clone()),
        ("--window-fg-color", ink.clone()),
        ("--view-bg-color", ground.clone()),
        ("--view-fg-color", ink.clone()),
        ("--headerbar-bg-color", ground.clone()),
        ("--headerbar-fg-color", ink.clone()),
        ("--headerbar-border-color", hairline.clone()),
        ("--headerbar-backdrop-color", ground.clone()),
        ("--headerbar-shade-color", hairline.clone()),
        ("--headerbar-darker-shade-color", hairline.clone()),
        ("--sidebar-bg-color", ground.clone()),
        ("--sidebar-fg-color", ink.clone()),
        ("--sidebar-backdrop-color", ground.clone()),
        ("--sidebar-border-color", hairline.clone()),
        ("--sidebar-shade-color", hairline.clone()),
        ("--secondary-sidebar-bg-color", surface.clone()),
        ("--secondary-sidebar-fg-color", ink.clone()),
        ("--secondary-sidebar-backdrop-color", ground.clone()),
        ("--secondary-sidebar-border-color", hairline.clone()),
        ("--secondary-sidebar-shade-color", hairline.clone()),
        ("--card-bg-color", surface.clone()),
        ("--card-fg-color", ink.clone()),
        ("--card-shade-color", hairline.clone()),
        ("--dialog-bg-color", surface.clone()),
        ("--dialog-fg-color", ink.clone()),
        ("--popover-bg-color", surface.clone()),
        ("--popover-fg-color", ink.clone()),
        ("--popover-shade-color", hairline.clone()),
        ("--thumbnail-bg-color", surface.clone()),
        ("--thumbnail-fg-color", ink.clone()),
        ("--overview-bg-color", surface.clone()),
        ("--overview-fg-color", ink.clone()),
        ("--shade-color", hairline.clone()),
        ("--scrollbar-outline-color", ground.clone()),
        ("--accent-bg-color", accent.clone()),
        ("--accent-fg-color", ground.clone()),
    ])
}

fn light_hc_roles(t: &Tokens) -> Result<Vec<(&'static str, String)>, TokenError> {
    Ok(vec![
        ("--postio-hairline", t.var("color-neutral-400")?),
        ("--postio-hairline-strong", t.var("color-neutral-600")?),
        ("--postio-dim", t.tint("color-text", 90.0)?),
        ("--postio-faint", t.tint("color-text", 80.0)?),
        ("--postio-accent-text", t.var("color-accent-800")?),
        ("--postio-selected-bg", t.tint("color-accent", 20.0)?),
        ("--postio-selected-strong-bg", t.tint("color-accent", 24.0)?),
        ("--postio-selected-accent-text", t.var("color-accent-900")?),
        ("--postio-key-fg", t.var("color-neutral-700")?),
        ("--postio-key-border", t.var("color-neutral-500")?),
        ("--headerbar-border-color", t.var("color-neutral-400")?),
        ("--sidebar-border-color", t.var("color-neutral-400")?),
        (
            "--secondary-sidebar-border-color",
            t.var("color-neutral-400")?,
        ),
        ("--card-shade-color", t.var("color-neutral-400")?),
        ("--shade-color", t.var("color-neutral-400")?),
        ("--accent-bg-color", t.var("color-accent-700")?),
    ])
}

fn dark_hc_roles(t: &Tokens) -> Result<Vec<(&'static str, String)>, TokenError> {
    Ok(vec![
        ("--postio-hairline", t.var("color-neutral-500")?),
        ("--postio-hairline-strong", t.var("color-neutral-400")?),
        ("--postio-dim", t.var("color-neutral-200")?),
        ("--postio-faint", t.var("color-neutral-300")?),
        ("--postio-accent-text", t.var("color-accent-200")?),
        ("--postio-selected-bg", t.var("color-accent-800")?),
        ("--postio-selected-strong-bg", t.var("color-accent-800")?),
        ("--postio-selected-accent-text", t.var("color-accent-200")?),
        ("--postio-key-fg", t.var("color-neutral-200")?),
        ("--postio-key-border", t.var("color-neutral-400")?),
        ("--headerbar-border-color", t.var("color-neutral-500")?),
        ("--sidebar-border-color", t.var("color-neutral-500")?),
        (
            "--secondary-sidebar-border-color",
            t.var("color-neutral-500")?,
        ),
        ("--card-shade-color", t.var("color-neutral-500")?),
        ("--shade-color", t.var("color-neutral-500")?),
        ("--accent-bg-color", t.var("color-accent-300")?),
    ])
}

/// The source shadows are ink-tinted for a paper ground. On a dark ground the
/// design system calls for ambient darkness instead, so the same geometry is
/// re-tinted with the neutral deep step at a heavier alpha.
fn dark_shadow(t: &Tokens, name: &str) -> Result<String, TokenError> {
    let value = t.need(name)?.to_string();
    let deep = parse_hex(t.need("color-neutral-900")?)
        .ok_or_else(|| TokenError("`--color-neutral-900` is not a hex colour".into()))?;
    // The normalised token looks like `0 3px 10px rgba(r, g, b, a)`.
    let Some(open) = value.find("rgba(") else {
        return err(format!("`--{name}` has no rgba() colour to re-tint"));
    };
    let Some(close) = value[open..].find(')') else {
        return err(format!("`--{name}` has an unterminated rgba()"));
    };
    let inner = &value[open + 5..open + close];
    let alpha: f32 = match inner.rsplit(',').next().map(|a| a.trim().parse()) {
        Some(Ok(a)) => a,
        _ => return err(format!("`--{name}` has no readable alpha")),
    };
    Ok(format!(
        "{}{}",
        &value[..open],
        rgba(deep, (alpha * 2.2).min(0.75))
    ))
}

// ── value normalisation ───────────────────────────────────────────────────

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

/// Split on `;` at paren depth zero, so `color-mix(a, b)` survives intact.
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

/// GTK CSS is a subset of web CSS. Fold the pieces it would choke on, or that
/// would silently resolve to something else, into forms it understands.
fn normalise_value(value: &str) -> Result<String, TokenError> {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let folded = fold_color_mix(&collapsed)?;
    Ok(drop_web_font_families(&folded))
}

/// `color-mix(in srgb, #rrggbb N%, transparent)` -> `rgba(r, g, b, 0.N)`.
///
/// GTK 4.16+ parses `color-mix()` itself, but folding it here keeps the
/// generated sheet free of anything version-dependent and makes the values
/// readable when someone opens `tokens.css` to see what a token became.
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

/// `system-ui` is a web keyword; GTK would treat it as a family name and hand
/// the row to whatever font happens to answer to it.
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

/// Two decimals is enough for an alpha and keeps the output byte-stable.
fn rgba((r, g, b): (u8, u8, u8), alpha: f32) -> String {
    let alpha = (alpha * 100.0).round() / 100.0;
    format!("rgba({r}, {g}, {b}, {alpha})")
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
}
