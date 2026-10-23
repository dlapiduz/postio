//! Focus GTK suite (fixture): a custom harness.
mod pickers;

const CASES: &[(&str, fn())] = &[
    (
        "pickers::s_then_2_snoozes",
        pickers::s_then_2_snoozes as fn(),
    ),
    (
        "pickers::skips_without_a_display",
        pickers::skips_without_a_display as fn(),
    ),
];
