//! Canvas 1b's row geometry, per density.
//!
//! Plain numbers in logical pixels, keyed off `postio_config::Density`. They
//! live here rather than in either frontend because "compact" has to mean the
//! same thing in both: a row that is 26px on one platform and 34 on the other
//! is not a shared setting, it is two settings with one name in the file.
//!
//! Type and colour are not here — those come from each toolkit's cascade.
//! This is only the layout the row arranges them in.

use postio_config::Density;

/// Canvas 1b's row geometry for one density, in logical pixels.
///
/// Type and colour come from the cascade ([`Palette`]); this is the layout
/// the snapshot arranges them in, which a hand-drawn widget owns the way a
/// `GtkBox` owns its spacing. The airy numbers are measured straight off the
/// canvas; the other two tighten the same anatomy rather than changing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Space above and below the row's content.
    pub pad_y: f32,
    /// How far in the content starts, accent edge included, so a row does
    /// not shift sideways when the selection lands on it.
    pub inset: f32,
    /// The avatar chip, square.
    pub avatar: f32,
    /// Between the avatar and the text column.
    pub gap: f32,
    /// Between the sender line and the subject.
    pub subject_gap: f32,
    /// Between the snippet and the key hints the focused row reveals.
    pub hints_gap: f32,
    /// Whether the snippet line is drawn at all.
    pub snippet: bool,
}

impl Metrics {
    /// The geometry `density` asks for.
    pub fn for_density(density: Density) -> Self {
        match density {
            Density::Airy => Metrics {
                pad_y: 11.0,
                inset: 21.0,
                avatar: 30.0,
                gap: 12.0,
                subject_gap: 3.0,
                hints_gap: 7.0,
                snippet: true,
            },
            Density::Comfortable => Metrics {
                pad_y: 8.0,
                inset: 18.0,
                avatar: 26.0,
                gap: 10.0,
                subject_gap: 2.0,
                hints_gap: 5.0,
                snippet: true,
            },
            // The tightest setting is for triage, where the question is how
            // many subjects fit on screen. The snippet is the line that
            // costs the most and answers it least.
            Density::Compact => Metrics {
                pad_y: 5.0,
                inset: 15.0,
                avatar: 22.0,
                gap: 9.0,
                subject_gap: 1.0,
                hints_gap: 4.0,
                snippet: false,
            },
        }
    }
}
