//! The colour a label's dot is drawn in: its own if it has one, else a hue from a wheel of
//! `HUES` chosen by name, so it is the same on every row and in both apps.
//! Never the accent's hue: a hue within `ACCENT_BAND` of the accent steps round the wheel, away
//! from it.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// `None` for anything else, which the caller treats as a label with no colour of its own.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let digits = hex.strip_prefix('#')?;
        if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |from: usize, len: usize| {
            let value = u8::from_str_radix(&digits[from..from + len], 16).ok()?;
            Some(if len == 1 { value * 17 } else { value })
        };
        match digits.len() {
            3 => Some(Self::new(channel(0, 1)?, channel(1, 1)?, channel(2, 1)?)),
            6 => Some(Self::new(channel(0, 2)?, channel(2, 2)?, channel(4, 2)?)),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// A grey has no hue and reads as 0.
    pub fn hue(self) -> f64 {
        let [r, g, b] = [self.r, self.g, self.b].map(|c| f64::from(c) / 255.0);
        let max = r.max(g).max(b);
        let delta = max - r.min(g).min(b);
        if delta == 0.0 {
            return 0.0;
        }
        let hue = if max == r {
            60.0 * (((g - b) / delta) % 6.0)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        hue.rem_euclid(360.0)
    }

    /// The conversion `tokens` makes, copied because that module is compiled alone by a build
    /// script.
    fn from_hsl(hue: f64, saturation: f64, lightness: f64) -> Self {
        let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
        let sector = hue.rem_euclid(360.0) / 60.0;
        let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
        let (r, g, b) = match sector as u32 {
            0 => (chroma, second, 0.0),
            1 => (second, chroma, 0.0),
            2 => (0.0, chroma, second),
            3 => (0.0, second, chroma),
            4 => (second, 0.0, chroma),
            _ => (chroma, 0.0, second),
        };
        let lift = lightness - chroma / 2.0;
        let byte = |v: f64| ((v + lift) * 255.0).round().clamp(0.0, 255.0) as u8;
        Self::new(byte(r), byte(g), byte(b))
    }
}

pub const ACCENT_BAND: f64 = 30.0;

pub const HUES: usize = 12;

const FIRST_HUE: f64 = 15.0;
const SATURATION: f64 = 0.5;
const LIGHTNESS: f64 = 0.42;

pub fn hue_distance(a: f64, b: f64) -> f64 {
    let apart = (a - b).rem_euclid(360.0);
    apart.min(360.0 - apart)
}

/// `stored` as is when the label has one; otherwise its wheel colour, moved to the nearest hue
/// outside `ACCENT_BAND`, away from the accent. Names compare ignoring case and surrounding space.
pub fn label_colour(name: &str, stored: Option<Rgb>, accent_hue: f64) -> Rgb {
    if let Some(stored) = stored {
        return stored;
    }
    let home = home(name);
    let clear = |index: usize| hue_distance(wheel(index).hue(), accent_hue) >= ACCENT_BAND;
    if clear(home) {
        return wheel(home);
    }
    let past = (wheel(home).hue() - accent_hue).rem_euclid(360.0) < 180.0;
    (1..HUES)
        .map(|steps| {
            if past {
                (home + steps) % HUES
            } else {
                (home + HUES - steps) % HUES
            }
        })
        .find(|index| clear(*index))
        .map_or_else(|| wheel(home), wheel)
}

fn wheel(index: usize) -> Rgb {
    let hue = FIRST_HUE + 360.0 / HUES as f64 * index as f64;
    Rgb::from_hsl(hue, SATURATION, LIGHTNESS)
}

/// FNV-1a over the folded bytes, so a name lands on the same hue in every build, which a
/// `HashMap`'s hasher does not promise.
fn home(name: &str) -> usize {
    let folded = name.trim().to_lowercase();
    let hash = folded
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    (hash % HUES as u64) as usize
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn natural(name: &str) -> Rgb {
        wheel(home(name))
    }

    const NAMES: &[&str] = &[
        "Receipts",
        "Travel",
        "Atlas",
        "Newsletters",
        "School",
        "Family",
        "Taxes",
        "Invoices",
        "Work",
        "Personal",
        "Follow up",
        "Waiting",
        "Read later",
        "Projects/Atlas",
        "Projects/Harbour",
        "2026",
        "a",
        "b",
        "Été",
        "日本",
        "Bills",
        "Health",
        "Car",
        "House",
        "Garden",
        "Books",
        "Music",
        "Photos",
        "Friends",
        "Club",
    ];

    fn accents() -> impl Iterator<Item = f64> {
        (0..120).map(|step| f64::from(step) * 3.0)
    }

    #[test]
    fn a_stored_colour_is_drawn_as_it_is() {
        // A stored colour is used as is, even inside the accent's band.
        let stored = Rgb::new(0x59, 0x80, 0xa6);
        for accent in [30.0, stored.hue()] {
            assert_eq!(label_colour("Receipts", Some(stored), accent), stored);
        }
    }

    #[test]
    fn a_stored_colour_reads_and_writes_as_hex() {
        assert_eq!(Rgb::from_hex("#5980a6"), Some(Rgb::new(0x59, 0x80, 0xa6)));
        assert_eq!(Rgb::from_hex("#5980A6"), Some(Rgb::new(0x59, 0x80, 0xa6)));
        assert_eq!(Rgb::from_hex("#58a"), Some(Rgb::new(0x55, 0x88, 0xaa)));
        assert_eq!(Rgb::new(0x59, 0x80, 0xa6).to_hex(), "#5980a6");
        for nonsense in ["", "#", "5980a6", "#5980a", "#5980a6ff", "#gggggg", "blue"] {
            assert_eq!(Rgb::from_hex(nonsense), None, "{nonsense:?}");
        }
    }

    #[test]
    fn hues_go_round_the_wheel() {
        assert_eq!(Rgb::new(255, 0, 0).hue(), 0.0);
        assert_eq!(Rgb::new(0, 255, 0).hue(), 120.0);
        assert_eq!(Rgb::new(0, 0, 255).hue(), 240.0);
        assert_eq!(Rgb::new(255, 0, 255).hue(), 300.0);
        assert_eq!(hue_distance(350.0, 10.0), 20.0);
        assert_eq!(hue_distance(10.0, 350.0), 20.0);
        assert_eq!(hue_distance(0.0, 180.0), 180.0);
        assert_eq!(hue_distance(90.0, 90.0), 0.0);
    }

    #[test]
    fn a_label_without_one_keeps_the_same_colour() {
        for accent in [0.0, 213.0] {
            let colour = label_colour("Receipts", None, accent);
            assert_eq!(label_colour("Receipts", None, accent), colour);
            assert_eq!(label_colour("receipts", None, accent), colour);
            assert_eq!(label_colour(" RECEIPTS ", None, accent), colour);
        }
    }

    #[test]
    fn a_label_without_one_is_never_drawn_in_the_accents_hue() {
        for accent in accents() {
            for name in NAMES {
                let hue = label_colour(name, None, accent).hue();
                assert!(
                    hue_distance(hue, accent) >= ACCENT_BAND,
                    "{name:?} is drawn at {hue} beside an accent at {accent}"
                );
            }
        }
    }

    #[test]
    fn labels_spread_over_the_palette() {
        for accent in [0.0, 213.0] {
            let colours: HashSet<Rgb> = NAMES
                .iter()
                .map(|name| label_colour(name, None, accent))
                .collect();
            assert!(
                colours.len() >= 6,
                "{} labels share {} colours",
                NAMES.len(),
                colours.len()
            );
        }
    }

    #[test]
    fn the_accent_moves_only_the_labels_it_would_collide_with() {
        for accent in accents() {
            for name in NAMES {
                let home = natural(name);
                let drawn = label_colour(name, None, accent);
                if hue_distance(home.hue(), accent) >= ACCENT_BAND {
                    assert_eq!(drawn, home, "{name:?} moved for an accent at {accent}");
                } else {
                    assert_ne!(drawn, home, "{name:?} stayed on an accent at {accent}");
                }
            }
        }
    }
}
