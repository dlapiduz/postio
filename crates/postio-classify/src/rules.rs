//! The data the rules layer reads (`contracts/engine.md`, "The classifier").

/// Everything the classifier decides by, as data rather than code
/// (constitution VII: providers are data):
///
/// - the automated-senders table, shipped as TOML (T105);
/// - the user's digest rules, from `[[focus.digests]]` (T131, T133);
/// - the marker kinds the user stopped, from `[focus.filter] stop_markers`
///   (T118);
/// - the needs-action detector's rules (T116).
///
/// It holds none of them yet: the skeleton classifies nothing, and each of
/// those tasks adds what its layer reads here, so a rule can only arrive as
/// data the pipeline is handed.
pub trait Rules {}
