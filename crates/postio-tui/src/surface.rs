//! What a click on one of Focus's surfaces lands on: a part of the surface
//! and its index, recorded by each surface as it draws.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    FilteredTab,
    FilteredRow,
    ConfirmCancel,
    ConfirmYes,
    DigestTab,
    DigestRow,
    DigestReference,
    RuleRow,
    RemoveCancel,
    RemoveConfirm,
    RuleField,
    RuleCancel,
    RuleCreate,
    CaptureMode,
    CapturePick,
    CaptureProject,
    CaptureText,
    CaptureCancel,
    ThreadChip,
    ForwardCard,
}

#[derive(Debug, Default)]
pub struct Surfaces {
    pub filtered: Option<crate::filtered::Filtered>,
    pub confirm: Option<postio_focus::Confirm>,
    pub digest: Option<crate::digest::Window>,
    pub rules: Option<crate::rules::Rules>,
    pub rule: Option<crate::rule_dialog::Form>,
    pub capture: Option<crate::capture::Sheet>,
}
