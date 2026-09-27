//! Legibility in every theme: the colour arithmetic and the rule of spec 006
//! research R10, as pure functions.
//!
//! The engine's part is to supply facts — each text run's colour and the
//! colour actually painted behind it, and what a message declares about its
//! own backgrounds and dark support — and to apply what this returns. The
//! rule itself is kept apart from the engine, where it is proven in
//! milliseconds with no display. It lives in this crate rather than in
//! `postio-ui` because `postio-ui` links `postio-core`, and with it `tokio`,
//! which the renderer's graph may not contain (FR-001).
//!
//! **The floor has no large-text allowance** (spec FR-012): 4.5:1 for every
//! run of text, 7:1 in high contrast.

/// An sRGB colour, each channel in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    /// Red.
    pub r: f64,
    /// Green.
    pub g: f64,
    /// Blue.
    pub b: f64,
}

impl Rgb {
    /// From 8-bit channels.
    pub fn from_u8(r: u8, g: u8, b: u8) -> Rgb {
        Rgb {
            r: f64::from(r) / 255.0,
            g: f64::from(g) / 255.0,
            b: f64::from(b) / 255.0,
        }
    }

    /// As 8-bit channels, rounded.
    pub fn to_u8(self) -> [u8; 3] {
        let channel = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        [channel(self.r), channel(self.g), channel(self.b)]
    }

    fn linear(self) -> [f64; 3] {
        let decode = |c: f64| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        [decode(self.r), decode(self.g), decode(self.b)]
    }

    fn from_linear([r, g, b]: [f64; 3]) -> Rgb {
        let encode = |c: f64| {
            if c <= 0.003_130_8 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            }
        };
        Rgb {
            r: encode(r),
            g: encode(g),
            b: encode(b),
        }
    }
}

/// A colour in OKLCH: perceptual lightness, chroma and hue in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    /// Lightness, `0.0..=1.0`.
    pub l: f64,
    /// Chroma.
    pub c: f64,
    /// Hue, degrees.
    pub h: f64,
}

/// The theme a message is drawn in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Theme {
    /// The application is dark.
    pub dark: bool,
    /// The application is in high contrast.
    pub high_contrast: bool,
}

/// WCAG relative luminance.
pub fn relative_luminance(colour: Rgb) -> f64 {
    let [r, g, b] = colour.linear();
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG contrast ratio, `1.0..=21.0`.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// sRGB to OKLCH.
pub fn to_oklch(colour: Rgb) -> Oklch {
    let [r, g, b] = colour.linear();
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let lightness = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
    let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    Oklch {
        l: lightness,
        c: (a * a + bb * bb).sqrt(),
        h: bb.atan2(a).to_degrees().rem_euclid(360.0),
    }
}

/// OKLCH to sRGB, and whether it was inside the gamut before clamping.
pub fn from_oklch(colour: Oklch) -> (Rgb, bool) {
    let (a, b) = (
        colour.c * colour.h.to_radians().cos(),
        colour.c * colour.h.to_radians().sin(),
    );
    let l = (colour.l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m = (colour.l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s = (colour.l - 0.089_484_177_5 * a - 1.291_485_548 * b).powi(3);
    let linear = [
        4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
        -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
        -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701 * s,
    ];
    // Tight: near black a channel is a few thousandths, and a tolerance
    // that admits a slightly negative one lets clamping bend the hue.
    const EPSILON: f64 = 1e-7;
    let in_gamut = linear
        .iter()
        .all(|c| (-EPSILON..=1.0 + EPSILON).contains(c));
    let clamped = linear.map(|c| c.clamp(0.0, 1.0));
    (Rgb::from_linear(clamped), in_gamut)
}

/// The colour at `lightness` with `colour`'s hue, keeping as much of its
/// chroma as the sRGB gamut allows.
fn at_lightness(colour: Oklch, lightness: f64) -> Rgb {
    let wanted = Oklch {
        l: lightness,
        ..colour
    };
    let (rgb, in_gamut) = from_oklch(wanted);
    if in_gamut {
        return rgb;
    }
    let (mut fits, mut too_much) = (0.0, colour.c);
    for _ in 0..30 {
        let middle = (fits + too_much) / 2.0;
        if from_oklch(Oklch {
            c: middle,
            ..wanted
        })
        .1
        {
            fits = middle;
        } else {
            too_much = middle;
        }
    }
    from_oklch(Oklch { c: fits, ..wanted }).0
}

/// The contrast floor for `theme` (spec FR-012).
pub fn floor(theme: Theme) -> f64 {
    if theme.high_contrast { 7.0 } else { 4.5 }
}

/// `text` moved in OKLCH lightness only, toward the far end from `ground`,
/// until it meets `floor` against it. Unchanged if it already does.
pub fn repair(text: Rgb, ground: Rgb, floor: f64) -> Rgb {
    if contrast(text, ground) >= floor {
        return text;
    }
    let colour = to_oklch(text);
    // Toward whichever end reads better against this ground.
    let white = Rgb {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };
    let black = Rgb {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };
    let end = if contrast(white, ground) >= contrast(black, ground) {
        1.0
    } else {
        0.0
    };
    if contrast(at_lightness(colour, end), ground) < floor {
        // Nothing in this hue reaches the floor; the far end is the best
        // there is.
        return at_lightness(colour, end);
    }
    // The smallest move in lightness that meets the floor.
    let (mut short, mut enough) = (colour.l, end);
    for _ in 0..40 {
        let middle = (short + enough) / 2.0;
        if contrast(at_lightness(colour, middle), ground) >= floor {
            enough = middle;
        } else {
            short = middle;
        }
    }
    // Pixels are 8-bit: rounding can take a colour that meets the floor
    // exactly back under it. Step on toward the far end until the colour as
    // painted meets it, and answer with that colour.
    let step = if end > colour.l { 0.001 } else { -0.001 };
    let mut lightness = enough;
    loop {
        let [r, g, b] = at_lightness(colour, lightness).to_u8();
        let painted = Rgb::from_u8(r, g, b);
        if contrast(painted, ground) >= floor || (lightness - end).abs() < 0.001 {
            return painted;
        }
        lightness = (lightness + step).clamp(0.0, 1.0);
    }
}

/// A CSS colour as an engine's computed style reports it, and its alpha:
/// how the tests spell the palette they assert against.
#[cfg(test)]
fn parse_css_color(value: &str) -> Option<(Rgb, f64)> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        let digit = |i: usize, len: usize| u8::from_str_radix(hex.get(i..i + len)?, 16).ok();
        return match hex.len() {
            3 | 4 => {
                let short = |i| digit(i, 1).map(|d| d * 17);
                let alpha = if hex.len() == 4 {
                    f64::from(short(3)?) / 255.0
                } else {
                    1.0
                };
                Some((Rgb::from_u8(short(0)?, short(1)?, short(2)?), alpha))
            }
            6 | 8 => {
                let alpha = if hex.len() == 8 {
                    f64::from(digit(6, 2)?) / 255.0
                } else {
                    1.0
                };
                Some((
                    Rgb::from_u8(digit(0, 2)?, digit(2, 2)?, digit(4, 2)?),
                    alpha,
                ))
            }
            _ => None,
        };
    }
    for function in ["rgba(", "rgb("] {
        if let Some(inner) = value
            .strip_prefix(function)
            .and_then(|rest| rest.strip_suffix(')'))
        {
            let parts: Vec<&str> = inner
                .split([',', '/', ' '])
                .filter(|part| !part.is_empty())
                .collect();
            let channel = |part: &str| -> Option<f64> {
                match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f64>().ok().map(|p| p / 100.0),
                    None => part.parse::<f64>().ok().map(|v| v / 255.0),
                }
            };
            let alpha = match parts.get(3) {
                Some(part) => match part.strip_suffix('%') {
                    Some(percent) => percent.parse::<f64>().ok()? / 100.0,
                    None => part.parse::<f64>().ok()?,
                },
                None => 1.0,
            };
            if parts.len() < 3 {
                return None;
            }
            let rgb = Rgb {
                r: channel(parts[0])?,
                g: channel(parts[1])?,
                b: channel(parts[2])?,
            };
            return Some((rgb, alpha));
        }
    }
    named(&value).map(|rgb| (rgb, if value == "transparent" { 0.0 } else { 1.0 }))
}

/// The named colours mail actually uses, and `transparent`.
#[cfg(test)]
fn named(name: &str) -> Option<Rgb> {
    let [r, g, b] = match name {
        "transparent" | "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "gray" | "grey" => [128, 128, 128],
        "silver" => [192, 192, 192],
        "maroon" => [128, 0, 0],
        "navy" => [0, 0, 128],
        "purple" => [128, 0, 128],
        "teal" => [0, 128, 128],
        "olive" => [128, 128, 0],
        "yellow" => [255, 255, 0],
        "orange" => [255, 165, 0],
        "lime" => [0, 255, 0],
        "aqua" | "cyan" => [0, 255, 255],
        "fuchsia" | "magenta" => [255, 0, 255],
        "darkgray" | "darkgrey" => [169, 169, 169],
        "lightgray" | "lightgrey" => [211, 211, 211],
        "whitesmoke" => [245, 245, 245],
        _ => return None,
    };
    Some(Rgb::from_u8(r, g, b))
}

/// How a message is presented in the current theme (spec FR-013).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// Light theme: as authored, on its own canvas.
    Styled,
    /// Dark theme, and the sender declared a dark design: theirs.
    SenderDark,
    /// Dark theme, designed mail: on the sender's own light canvas, a sheet
    /// of paper inside the dark app.
    Paper,
    /// Paper the user asked to darken (FR-013a).
    Darkened,
    /// Dark theme, not designed: on the reader's ground, text repaired.
    Adapted,
}

/// What a message says about its own colours, as an engine reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MessageFacts {
    /// The page's background colour, lifted from `<body>`/`<html>` or a
    /// `body` rule, if the sender set one.
    pub canvas: Option<Rgb>,
    /// Whether any element inside the message, other than the page itself,
    /// declares a background colour or image.
    pub inner_background: bool,
    /// Whether the sender declared dark support: `color-scheme` naming
    /// `dark`, or a rule under `prefers-color-scheme: dark`.
    pub declares_dark: bool,
}

/// The relative luminance at and above which a page background is a
/// client's default white rather than a design (FR-013(c)).
pub const NEAR_WHITE: f64 = 0.9;

/// How a message with `facts` is presented in `theme` (FR-013).
///
/// In priority order: a light theme draws everything as authored; a
/// sender's own dark design wins; designed mail -- an inner background, or a
/// page that is not plain white -- is paper, or darkened paper if the user
/// asked; everything else adapts to the reader's ground. A desktop client
/// stamping a white page on every reply is not a design.
pub fn classify(facts: MessageFacts, theme: Theme, darkened: bool) -> Presentation {
    if !theme.dark {
        return Presentation::Styled;
    }
    if facts.declares_dark {
        return Presentation::SenderDark;
    }
    let designed = facts.inner_background
        || facts
            .canvas
            .is_some_and(|canvas| relative_luminance(canvas) < NEAR_WHITE);
    match (designed, darkened) {
        (true, true) => Presentation::Darkened,
        (true, false) => Presentation::Paper,
        (false, _) => Presentation::Adapted,
    }
}

/// A background colour remapped for a darkened message (FR-013a): OKLab
/// lightness into `[0.12, 0.30]`, inverted, with its hue kept.
pub fn darken(background: Rgb) -> Rgb {
    let colour = to_oklch(background);
    at_lightness(colour, 0.12 + (1.0 - colour.l.clamp(0.0, 1.0)) * 0.18)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(value: &str) -> Rgb {
        parse_css_color(value).expect("a colour").0
    }

    fn hue_distance(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    #[test]
    fn contrast_matches_wcag_at_known_values() {
        assert!((contrast(hex("#000000"), hex("#ffffff")) - 21.0).abs() < 1e-9);
        assert!((contrast(hex("#ffffff"), hex("#ffffff")) - 1.0).abs() < 1e-9);
        let grey = contrast(hex("#777777"), hex("#ffffff"));
        assert!(
            grey < 4.5 && grey > 4.4,
            "#777 on white is just under AA: {grey}"
        );
        assert!((relative_luminance(hex("#ffffff")) - 1.0).abs() < 1e-9);
        assert!(relative_luminance(hex("#000000")).abs() < 1e-9);
        // Symmetric: the order of the two colours does not matter.
        assert_eq!(
            contrast(hex("#123456"), hex("#abcdef")),
            contrast(hex("#abcdef"), hex("#123456"))
        );
    }

    #[test]
    fn oklch_round_trips() {
        for value in [
            "#000000", "#ffffff", "#c0392b", "#1d5c1d", "#2b2b2d", "#94bce3", "#ff00ff",
        ] {
            let colour = hex(value);
            let (back, in_gamut) = from_oklch(to_oklch(colour));
            assert!(in_gamut, "{value}");
            assert_eq!(back.to_u8(), colour.to_u8(), "{value}");
        }
    }

    #[test]
    fn the_floor_has_no_large_text_allowance() {
        assert_eq!(
            floor(Theme {
                dark: true,
                high_contrast: false
            }),
            4.5
        );
        assert_eq!(
            floor(Theme {
                dark: false,
                high_contrast: false
            }),
            4.5
        );
        assert_eq!(
            floor(Theme {
                dark: true,
                high_contrast: true
            }),
            7.0
        );
    }

    /// The bug the user reported, repaired: dark text on the dark reader
    /// ground, moved in lightness only until it reads.
    #[test]
    fn repair_meets_the_floor_in_lightness_only() {
        let ground = hex("#2b2b2d");
        for text in [
            "#333333", "#222222", "#1d5c1d", "#7a4b0c", "#253f7a", "#8a1c1c",
        ] {
            let before = hex(text);
            for wanted in [4.5, 7.0] {
                let after = repair(before, ground, wanted);
                let ratio = contrast(after, ground);
                assert!(ratio >= wanted - 1e-6, "{text} at {wanted}: {ratio}");
                let (was, now) = (to_oklch(before), to_oklch(after));
                if was.c > 0.02 && now.c > 0.02 {
                    assert!(
                        hue_distance(was.h, now.h) <= 2.0,
                        "{text}: hue {} -> {}",
                        was.h,
                        now.h
                    );
                }
                assert!(now.l > was.l, "{text}: on a dark ground, repair lightens");
            }
        }
        // On a light ground it darkens instead.
        let after = repair(hex("#bbbbbb"), hex("#ffffff"), 4.5);
        assert!(contrast(after, hex("#ffffff")) >= 4.5 - 1e-6);
        assert!(to_oklch(after).l < to_oklch(hex("#bbbbbb")).l);
    }

    /// Pixels are 8-bit. A repair that meets the floor only before rounding
    /// paints a colour that misses it: the evaluation harness found text at
    /// 4.48:1 that the arithmetic had put at exactly 4.5.
    #[test]
    fn a_repair_still_meets_the_floor_once_painted_in_8_bits() {
        let grounds = ["#2b2b2d", "#1e1e1e", "#ffffff", "#f5f5f8", "#fff4e0"];
        let texts = [
            "#000000", "#222222", "#333333", "#777777", "#1d5c1d", "#8a1c1c", "#bbbbbb",
        ];
        for ground in grounds {
            for text in texts {
                for floor in [4.5, 7.0] {
                    let [r, g, b] = repair(hex(text), hex(ground), floor).to_u8();
                    let painted = Rgb::from_u8(r, g, b);
                    let reachable = contrast(Rgb::from_u8(255, 255, 255), hex(ground))
                        .max(contrast(Rgb::from_u8(0, 0, 0), hex(ground)))
                        >= floor;
                    if reachable {
                        assert!(
                            contrast(painted, hex(ground)) >= floor,
                            "{text} on {ground} at {floor}: painted {:?} is {}",
                            [r, g, b],
                            contrast(painted, hex(ground))
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_colour_that_already_reads_is_left_alone() {
        for (text, ground) in [
            ("#f5f5f8", "#2b2b2d"),
            ("#1d1f20", "#ffffff"),
            ("#c0392b", "#ffffff"),
        ] {
            let text = hex(text);
            assert_eq!(repair(text, hex(ground), 4.5), text);
        }
    }

    /// A saturated colour lightened far enough leaves the sRGB gamut; chroma
    /// gives way rather than the floor.
    #[test]
    fn chroma_gives_way_only_when_the_gamut_runs_out() {
        let ground = hex("#2b2b2d");
        let blue = hex("#0000ff");
        let after = repair(blue, ground, 7.0);
        assert!(contrast(after, ground) >= 7.0 - 1e-6);
        assert!(
            to_oklch(after).c < to_oklch(blue).c,
            "chroma reduced to stay in gamut"
        );
        let within = hex("#1d5c1d");
        let after = repair(within, ground, 4.5);
        assert!(
            (to_oklch(after).c - to_oklch(within).c).abs() < 0.01,
            "chroma kept when it fits"
        );
    }

    /// A canvas whose relative luminance is exactly `target`.
    fn grey_at(target: f64) -> Rgb {
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..60 {
            let middle = (low + high) / 2.0;
            if relative_luminance(Rgb {
                r: middle,
                g: middle,
                b: middle,
            }) < target
            {
                low = middle;
            } else {
                high = middle;
            }
        }
        Rgb {
            r: high,
            g: high,
            b: high,
        }
    }

    #[test]
    fn every_message_is_presented_by_one_stated_rule() {
        let light = Theme::default();
        let dark = Theme {
            dark: true,
            high_contrast: false,
        };
        let white = Some(hex("#ffffff"));
        let facts = |canvas: Option<Rgb>, inner: bool, declares: bool| MessageFacts {
            canvas,
            inner_background: inner,
            declares_dark: declares,
        };
        for (what, message, theme, expected) in [
            (
                "light theme, anything",
                facts(white, true, false),
                light,
                Presentation::Styled,
            ),
            (
                "a declared dark design",
                facts(white, true, true),
                dark,
                Presentation::SenderDark,
            ),
            (
                "designed: an inner background",
                facts(None, true, false),
                dark,
                Presentation::Paper,
            ),
            (
                "designed: a coloured page",
                facts(Some(grey_at(0.89)), false, false),
                dark,
                Presentation::Paper,
            ),
            (
                "a near-white page is a default",
                facts(Some(grey_at(0.91)), false, false),
                dark,
                Presentation::Adapted,
            ),
            (
                "a white page is a default",
                facts(white, false, false),
                dark,
                Presentation::Adapted,
            ),
            (
                "nothing at all",
                facts(None, false, false),
                dark,
                Presentation::Adapted,
            ),
        ] {
            assert_eq!(classify(message, theme, false), expected, "{what}");
        }
        // Darken applies to paper, and to nothing else.
        assert_eq!(
            classify(facts(None, true, false), dark, true),
            Presentation::Darkened
        );
        assert_eq!(
            classify(facts(white, false, false), dark, true),
            Presentation::Adapted
        );
        assert_eq!(
            classify(facts(None, true, false), light, true),
            Presentation::Styled
        );
    }

    #[test]
    fn darken_lands_backgrounds_in_the_dark_band_with_hue_kept() {
        for value in [
            "#ffffff", "#f7e8d0", "#dcebe3", "#dde4f3", "#1f6fa8", "#000000",
        ] {
            let before = hex(value);
            let after = to_oklch(darken(before));
            assert!(
                (0.12 - 1e-6..=0.30 + 1e-6).contains(&after.l),
                "{value}: L = {}",
                after.l
            );
            let was = to_oklch(before);
            if was.c > 0.02 && after.c > 0.02 {
                assert!(hue_distance(was.h, after.h) <= 2.0, "{value}");
            }
        }
        // Inverted: the lightest page becomes the darkest ground.
        assert!(to_oklch(darken(hex("#ffffff"))).l < to_oklch(darken(hex("#1f6fa8"))).l);
    }

    #[test]
    fn css_colours_parse_as_engines_report_them() {
        assert_eq!(
            parse_css_color("#fff").map(|c| c.0.to_u8()),
            Some([255, 255, 255])
        );
        assert_eq!(
            parse_css_color("#0a0B0c").map(|c| c.0.to_u8()),
            Some([10, 11, 12])
        );
        assert_eq!(
            parse_css_color("rgb(1, 2, 3)").map(|c| c.0.to_u8()),
            Some([1, 2, 3])
        );
        let (colour, alpha) = parse_css_color("rgba(10, 20, 30, 0.5)").expect("rgba");
        assert_eq!(colour.to_u8(), [10, 20, 30]);
        assert!((alpha - 0.5).abs() < 1e-9);
        assert_eq!(parse_css_color("rgba(0, 0, 0, 0)").map(|c| c.1), Some(0.0));
        assert_eq!(
            parse_css_color("White").map(|c| c.0.to_u8()),
            Some([255, 255, 255])
        );
        assert_eq!(parse_css_color("transparent").map(|c| c.1), Some(0.0));
        assert_eq!(parse_css_color("not a colour"), None);
        assert_eq!(parse_css_color("#12"), None);
    }
}
