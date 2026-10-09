//! The boundary the macOS frontend talks to.
//!
//! Postio's engine is toolkit-free by construction — `postio-session` is the
//! composition root with no GTK in it, and `postio-core` is the
//! commands-down/events-up contract over it (ADR 0010, `ARCHITECTURE.md` §9).
//! This crate is what lets something that is not Rust hold the other end of
//! that contract: a UniFFI surface over `postio-session`, consumed by the
//! native Swift application (ADR 0019).
//!
//! # What this is not
//!
//! **It is private to the macOS app, and it promises no stability.** It exists
//! to serve one frontend and is shaped by that frontend's needs. It is not an
//! embedding API, and a caller outside this repository has no contract here.
//!
//! That sentence is load-bearing rather than defensive. Ghostty — the design
//! this seam is modelled on — says the same of `include/ghostty.h`, and its
//! genuinely public library ended up a separate artifact with separate
//! documentation. A frontend seam that accretes general-purpose obligations
//! stops being able to change, and this one has a lot of changing left to do.
//!
//! # Why it builds on Linux
//!
//! There is deliberately no macOS-specific code here, so `cargo test -p
//! postio-ffi` runs anywhere. That is the mechanism which keeps a Linux
//! session from breaking the macOS seam without noticing: the shim for the
//! expensive platform is compiled on the cheap one, in the ordinary gate.
//!
//! # The shape it is growing into
//!
//! Two tiers, after Ghostty's `apprt`. A small **required floor** — open a
//! session, drain events, invoke a command, page the list, render a reader
//! document — where a missing piece is a compile error. And a large
//! **optional surface** of one-way events, each ignorable, so a frontend that
//! handles none of them still runs. Today the floor is one function; the
//! scaffolding around it is the part that had to be proven first.

mod account;
mod compose;
mod contacts;
mod conversation;
mod cost;
mod demo;
mod dwell;
mod event;
mod finder;
mod focus;
mod focus_bar;
mod focus_compose;
mod focus_keymap;
mod focus_list;
mod focus_message;
mod focus_pickers;
mod focus_reader;
mod focus_search;
mod focus_states;
mod focus_surfaces;
mod keys;
mod links;
mod list;
mod logging;
mod mailbox;
mod notify;
mod palette;
mod parts;
mod provisioning;
mod reader;
pub mod registry;
mod saved_search;
mod search;
mod session;
mod settings;
mod unsubscribe;

pub use account::{AccountFfi, ConnectionReportFfi, RepairRouteFfi};
pub use compose::{
    AttachmentFfi, ComposeError, DraftFfi, DraftKindFfi, InlineImageFfi, PastedFfi, QuoteFoldFfi,
    composer_title, draft_saved_words, draft_summary, fold_quote, outgoing_shape,
    recipient_summary, remind_meaning, remind_presets,
};
pub use contacts::{ExternalContactFfi, RecipientSuggestionFfi};
pub use conversation::{
    ConversationFfi, ThreadAnchorFfi, ThreadDocumentFfi, ThreadVerbFfi, ThreadVerbKindFfi,
    message_when, thread_expand_all_script, thread_observer_script, thread_scroll_script,
    thread_toggle_script, thread_verb,
};
pub use cost::{
    note_reader_render, note_reader_surface_created, note_reader_surface_released,
    reader_renders_issued, reader_surfaces_created, reader_surfaces_held,
};
pub use dwell::{DwellArmFfi, dwell_on_cursor};
pub use event::{ConnectionStateFfi, FailureReasonFfi, NoticeKindFfi, ToastKindFfi, UiEvent};
pub use finder::{FinderAnswerFfi, FinderHitFfi};
pub use focus::{FocusCountsFfi, FocusStripFfi};
pub use focus_bar::{
    BarLineFfi, BarLineKindFfi, BarModeFfi, BarSelectFfi, BarViewFfi, PlaceEntryFfi, PlaceMarkFfi,
};
pub use focus_compose::ComposerKindFfi;
pub use focus_keymap::{KeyMapGroupFfi, KeyMapRowFfi, KeyMapSheetFfi};
pub use focus_list::{
    FocusRowActionFfi, FocusRowFfi, FocusRowKindFfi, FocusScopeFfi, LabelPillFfi, MarkerLineFfi,
    ReaderVerbFfi, SurfaceKindFfi,
};
pub use focus_message::{
    FocusAttachmentFfi, FocusFieldFfi, FocusMessageViewFfi, FocusPersonFfi, FocusThreadChipFfi,
    FocusVerbFfi,
};
pub use focus_pickers::{
    PickerAnchorFfi, PickerFieldFfi, PickerKindFfi, PickerRowFfi, PickerViewFfi,
};
pub use focus_reader::{FocusReaderDocumentFfi, RenderModeWordsFfi, TreatmentFfi};
pub use focus_search::{
    ConversationOrderFfi, DropdownRowFfi, DropdownRowKindFfi, DropdownSectionFfi, DropdownStateFfi,
    DropdownViewFfi, FilterButtonFfi, FilterKindFfi, MonthBarFfi, PillFfi, QueryChipFfi,
    QueryViewFfi, ResultGroupFfi, ResultRowFfi, ResultsTabFfi, ResultsViewFfi, RunFfi, RunStyleFfi,
    TabFfi, TermEditFfi,
};
pub use focus_states::{
    BannerButtonFfi, BannerFfi, BannerProgressFfi, EmptyPageFfi, EmptyShortcutFfi, SyncMarkFfi,
};
pub use focus_surfaces::{
    CaptureModeFfi, CapturePickFfi, CaptureProjectFfi, CaptureViewFfi, CapturedFfi, ConfirmFfi,
    DigestCardFfi, DigestEmailFfi, DigestLineFfi, DigestPageFfi, DigestStatementFfi,
    DigestSummaryFfi, DigestTopicFfi, DigestViewFfi, FilteredLineFfi, FilteredTabFfi,
    FilteredViewFfi, FocusDigestGeometryFfi, FocusHintFfi, RulePreviewLineFfi, RuleScheduleFfi,
    RuleViewFfi, SummaryStatementFfi, VaultPictureFfi, VaultProjectFfi, focus_digest_geometry,
};
pub use keys::{KeyOutcomeFfi, ModifiersFfi};
pub use links::{link_gone, link_unknown, message_link, parse_message_link};
pub use list::{RowFfi, ScopeFfi};
pub use logging::start_logging;
pub use mailbox::{MailboxFfi, MailboxRoleFfi, mailbox_role_name};
pub use notify::{
    MailArrivalFfi, MailNotificationFfi, NotificationDecisionFfi, SuppressedFfi,
    decide_notification,
};
pub use palette::{CheatRowFfi, CheatSectionFfi, PaletteEntryFfi};
pub use parts::PartsError;
pub use provisioning::{
    DiscoveredFfi, NewAccountFfi, ProviderHintFfi, RouteFfi, ScopesFfi, SecurityFfi, ServerFfi,
    SignInProgressFfi, SyncWindowChoiceFfi, SyncWindowFfi, looks_like_an_address, provider_hint,
    sign_in_scopes, sync_window_choices, write_initial_sync_window,
};
pub use reader::{
    ConversationActionFfi, GrantFfi, InlinePart, MessageFactsFfi, ReaderActionFfi,
    ReaderDocumentFfi, ReaderNoticeFfi, RecipientsFfi, RemoteImagesFfi, middle_truncate,
    reader_page_after, reader_page_fragment, reader_scroll_markers,
};
pub use registry::{CommandSpecFfi, MenuFfi, MenuSectionFfi, UiContext, UiRecovery, menus};
pub use saved_search::{SavedSearchEditFfi, SavedSearchFfi, save_search, saved_searches};
pub use search::{ChipFfi, MatchRangeFfi, SnippetFfi, query_chips};
pub use session::{Session, SessionError, SessionOptions, StartedOverFfi, start_over_with};
pub use settings::{
    AppearanceFfi, AttachmentFetchFfi, BodyFetchFfi, CheckForMailFfi, ComposingFfi, DensityFfi,
    FilterFfi, FilteringEntryFfi, FilteringPageFfi, FilteringUndoFfi, FoundEditorFfi, GroupFfi,
    HandoffTargetFfi, KeyHintFfi, RowActionFfi, RowMetricsFfi, SettingsError, SettingsSectionFfi,
    SettingsStatusFfi, SignaturePlacementFfi, SyncingFfi, ThemeFfi, row_actions, row_metrics,
    row_timestamp, settings_add_filter, settings_appearance, settings_composing,
    settings_filtering, settings_filters, settings_group_label, settings_handoff_label,
    settings_handoff_target, settings_humanize_interval, settings_load, settings_patch_appearance,
    settings_patch_composing, settings_patch_filter, settings_patch_filtering,
    settings_patch_syncing, settings_path, settings_remove_filter, settings_save,
    settings_sections, settings_status, settings_syncing, settings_take_back_filter,
};
pub use unsubscribe::{UnsubscribeActivationFfi, UnsubscribeOfferFfi};

/// Every command the registry knows, in cheat-sheet order.
///
/// A free function because the registry is a `const` table: it is not session
/// state, and requiring a session to read it would be an accident of where the
/// method happens to live.
///
/// That matters more on macOS than it looks. Opening a session reads the
/// store's key from the OS keyring (ADR 0014), and an unsigned build has a new
/// code identity on every rebuild — so anything that needed a session to show
/// a command list would raise a Keychain prompt to draw a menu. The frontend
/// builds its palette, cheat sheet and menu bar from this, before it has a
/// store or a single secret.
#[uniffi::export]
pub fn commands() -> Vec<CommandSpecFfi> {
    registry::commands()
}

/// The ids `PostioKit.Intercepted` must hold, as the boundary knows them.
///
/// Crosses so the Swift copy can be checked against it rather than trusted.
/// See [`registry::INTERCEPTED`] for why there are two copies at all.
#[uniffi::export]
pub fn intercepted_commands() -> Vec<String> {
    registry::INTERCEPTED
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// Which of Postio's interfaces the Mac app is, to the registry: Focus
/// (specs/009-focus-macos FR-001, ADR 0043). What it is offered -- keys,
/// palette rows, menu items, settings sections -- is Focus's, less what
/// `postio_core::registry::offered_on` keeps off the Mac.
pub const FRONTEND: postio_core::Frontend = postio_core::Frontend::Focus;

uniffi::setup_scaffolding!();

/// Set aside the store at `store_path` (or the usual path) and start a fresh
/// one there, carrying its accounts across: what "Start over" does when
/// [`Session::open_at`] answers [`SessionError::StoreFromAnotherBuild`]
/// (specs/009-focus-macos T098). The store key is the keyring's. Blocks, as
/// opening a session does: call it off the main actor, then open again.
#[uniffi::export]
pub fn start_over(store_path: Option<String>) -> Result<StartedOverFfi, SessionError> {
    start_over_with(match store_path {
        Some(path) => SessionOptions::at(path),
        None => SessionOptions::at_default_path(),
    })
}

/// The words of the page a store that will not open shows
/// (specs/009-focus-macos T100), `postio_ui::focus_state`'s, as GTK's
/// window says them.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StoreRefusalWordsFfi {
    /// The heading when trying again can help.
    pub cant_open: String,
    /// Its button.
    pub try_again: String,
    /// The heading for [`SessionError::StoreFromAnotherBuild`].
    pub from_another_build: String,
    /// What that page says: why, what a fresh store keeps, what stays.
    pub start_over_sentence: String,
    /// Its button, which runs [`start_over`].
    pub start_over: String,
    /// The button while [`start_over`] runs.
    pub starting_over: String,
}

/// The refusal page's words. No session: there is none while it shows.
#[uniffi::export]
pub fn store_refusal_words() -> StoreRefusalWordsFfi {
    use postio_ui::focus_state as words;
    StoreRefusalWordsFfi {
        cant_open: words::CANT_OPEN_MAIL.to_owned(),
        try_again: words::TRY_AGAIN.to_owned(),
        from_another_build: words::STORE_FROM_ANOTHER_VERSION.to_owned(),
        start_over_sentence: words::START_OVER.to_owned(),
        start_over: words::START_A_FRESH_STORE.to_owned(),
        starting_over: words::STARTING_A_FRESH_STORE.to_owned(),
    }
}

/// What is said once [`start_over`] has set the old store aside at
/// `set_aside` and the fresh one has opened.
#[uniffi::export]
pub fn started_over_words(set_aside: String) -> String {
    postio_ui::focus_state::started_over(&set_aside)
}

/// Answers with the name of this application.
///
/// A deliberate placeholder: the boundary needs one real export before it has
/// any exports at all, and every guarantee this crate makes — that the
/// scaffolding compiles under the workspace's `forbid(unsafe_code)`, that
/// `uniffi-bindgen` can read the cdylib's metadata, that the generated Swift
/// declares what Rust exported — is proven against it. It is replaced by the
/// real floor, not extended.
#[uniffi::export]
pub fn probe() -> String {
    "postio".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_is_callable_as_ordinary_rust() {
        // The scaffolding must not change what the function is from Rust's
        // side: `#[uniffi::export]` adds a C-ABI shim beside it, and a version
        // that wrapped or moved the original would break every in-process
        // caller and every test in this workspace.
        assert_eq!(probe(), "postio");
    }
}
