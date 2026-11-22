//! The boundary the macOS frontend talks to: a UniFFI surface over `postio-session`, private to the
//! macOS app with no stability promise (ADR 0019).
//! It has no macOS-specific code, so `cargo test -p postio-ffi` runs on Linux.

mod account;
mod agent_access;
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
mod focus_contact_edit;
mod focus_contact_merge;
mod focus_contacts;
mod focus_contacts_io;
mod focus_contacts_settings;
mod focus_contacts_sync;
mod focus_keymap;
mod focus_list;
mod focus_message;
mod focus_pickers;
mod focus_reader;
mod focus_search;
mod focus_states;
mod focus_surfaces;
mod focus_thread_bar;
mod focus_threads;
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
mod stack;
mod unsubscribe;

pub use account::{AccountFfi, ConnectionReportFfi, RepairRouteFfi};
pub use agent_access::{
    AgentActivityFfi, AgentCallFfi, AgentWordsFfi, SharedAgentAccountFfi, settings_agent_sharing,
    settings_agent_words, settings_patch_revoke_agent, settings_patch_revoke_all_agents,
};
pub use compose::{
    AttachmentFfi, ComposeError, DraftFfi, DraftKindFfi, DraftLabelFfi, InlineImageFfi, PastedFfi,
    QuoteFoldFfi, composer_title, draft_saved_words, draft_summary, fold_quote, outgoing_shape,
    recipient_summary, remind_meaning, remind_presets,
};
pub use contacts::{ContactFfi, ContactKindFfi, ExternalContactFfi, RecipientSuggestionFfi};
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
pub use focus::{FocusCountsFfi, FocusStripFfi, NoticesFfi};
pub use focus_bar::{
    BarLineFfi, BarLineKindFfi, BarModeFfi, BarSelectFfi, BarViewFfi, PlaceEntryFfi, PlaceMarkFfi,
};
pub use focus_compose::{
    BarActionFfi, ChooserCardFfi, ChooserFfi, ComposerKindFfi, DraftEditFfi, ForwardFormFfi,
    NewerMessageFfi, OlderReplyFfi,
};
pub use focus_contact_edit::{
    AddressLabelFfi, ContactEditViewFfi, EditAddressFfi, EditButtonFfi, EditChangeFfi, EditHintFfi,
    EditSeenFfi, EditSegmentFfi, FormFieldFfi, GroupOptionFfi, GroupPickerFfi, ShowAsFfi,
};
pub use focus_contact_merge::{MergeCardFfi, MergeResultFfi, MergeSheetViewFfi};
pub use focus_contacts::{
    ArrivalOptionFfi, AvatarFfi, CardAddressFfi, CardButtonFfi, CardFileFfi, CardLineFfi,
    CardMonthsFfi, CardSwitchFfi, ContactArrivalFfi, ContactCardViewFfi, ContactPopoverViewFfi,
    ContactReviewLineFfi, ContactRowViewFfi, ContactTabFfi, ContactTabViewFfi, ContactsFooterFfi,
    ContactsStripFfi, ContactsViewFfi, OpenLineFfi, PopoverActionFfi, PopoverTileFfi,
};
pub use focus_contacts_io::{
    ContactsExportResultFfi, ContactsImportResultFfi, ContactsIoSheetFfi, ContactsPanelWordsFfi,
};
pub use focus_contacts_settings::{
    ContactsPaneFfi, SuggestionRowFfi, SuggestionsViewFfi, SyncModeFfi, SyncModeOptionFfi,
    SyncSectionFfi, SyncStatusFfi,
};
pub use focus_contacts_sync::{
    BringInFfi, BringInResultFfi, ExternalRecordFfi, SyncAccountFfi, SyncPlanFfi, SyncResultFfi,
    SyncScopeFfi, SyncScopeOptionFfi, SyncSheetFfi,
};
pub use focus_keymap::{KeyMapGroupFfi, KeyMapRowFfi, KeyMapSheetFfi};
pub use focus_list::{
    FocusRowActionFfi, FocusRowFfi, FocusRowKindFfi, FocusScopeFfi, LabelPillFfi, MarkerLineFfi,
    ReaderVerbFfi, SurfaceKindFfi,
};
pub use focus_message::{
    FocusAttachmentFfi, FocusFieldFfi, FocusMessageViewFfi, FocusPersonFfi, FocusVerbFfi,
    OpenChoicesFfi, OpenLinkFfi,
};
pub use focus_pickers::{
    PickerAnchorFfi, PickerFieldFfi, PickerKindFfi, PickerRowFfi, PickerViewFfi,
};
pub use focus_reader::{
    FocusMessageColumnFfi, FocusMessageOpenFfi, FocusReaderDocumentFfi, RenderModeWordsFfi,
    TreatmentFfi, focus_message_column,
};
pub use focus_search::{
    ConversationOrderFfi, DropdownRowFfi, DropdownRowKindFfi, DropdownSectionFfi, DropdownStateFfi,
    DropdownViewFfi, FileCardFfi, FileCopyFfi, FilePreviewFfi, FilesHeaderFfi, FilterButtonFfi,
    FilterKindFfi, MonthBarFfi, PersonRowFfi, PillFfi, QueryChipFfi, QueryViewFfi, ResultGroupFfi,
    ResultRowFfi, ResultsTabFfi, ResultsViewFfi, RunFfi, RunStyleFfi, SearchWordsFfi, TabFfi,
    TermEditFfi, focus_search_checked, focus_search_row_accessible, focus_search_words,
};
pub use focus_states::{
    BannerButtonFfi, BannerFfi, BannerProgressFfi, EmptyPageFfi, EmptyShortcutFfi, SyncMarkFfi,
};
pub use focus_surfaces::{
    CaptureModeFfi, CapturePickFfi, CaptureProjectFfi, CaptureViewFfi, CapturedFfi, ConfirmFfi,
    DigestCardFfi, DigestEmailFfi, DigestLineFfi, DigestPageFfi, DigestStatementFfi,
    DigestSummaryFfi, DigestTopicFfi, DigestViewFfi, FilteredLineFfi, FilteredSpliceFfi,
    FilteredTabFfi, FilteredViewFfi, FocusDigestGeometryFfi, FocusHintFfi, RulePreviewLineFfi,
    RuleScheduleFfi, RuleViewFfi, SummaryStatementFfi, VaultPictureFfi, VaultProjectFfi,
    focus_digest_geometry,
};
pub use focus_thread_bar::{
    OutlineFfi, OutlinePersonFfi, OutlineRowFfi, ThreadBannerFfi, ThreadBarFfi, ThreadChipFfi,
    ThreadMenuEntryFfi, ThreadMenuFfi, ThreadMenuGroupFfi,
};
pub use focus_threads::{
    ChevronFfi, ChildHintFfi, ChildRowFfi, FooterKeyFfi, FooterViewFfi, MatchBannerFfi,
    ResultChildFfi, ResultSpliceFfi, ThreadRowFfi,
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
    AfterUnsubscribeFfi, AppearanceFfi, AttachmentFetchFfi, AutoEmptyChoiceFfi, BodyFetchFfi,
    CheckForMailFfi, ComposingFfi, DensityFfi, FilterFfi, FilteringEntryFfi, FilteringPageFfi,
    FilteringUndoFfi, FoundEditorFfi, GroupFfi, HandoffTargetFfi, KeyHintFfi, RowActionFfi,
    RowMetricsFfi, SettingsError, SettingsSectionFfi, SettingsStatusFfi, SignaturePlacementFfi,
    SyncingFfi, ThemeFfi, row_actions, row_metrics, row_timestamp, settings_add_filter,
    settings_after_unsubscribe, settings_appearance, settings_auto_empty_choices,
    settings_auto_empty_words, settings_composing, settings_filtering, settings_filters,
    settings_group_label, settings_handoff_label, settings_handoff_target,
    settings_humanize_interval, settings_load, settings_patch_after_unsubscribe,
    settings_patch_appearance, settings_patch_composing, settings_patch_filter,
    settings_patch_filtering, settings_patch_syncing, settings_path, settings_remove_filter,
    settings_save, settings_sections, settings_status, settings_syncing, settings_take_back_filter,
};
pub use unsubscribe::{ListMailOfferFfi, UnsubscribeActivationFfi, UnsubscribeOfferFfi};

/// Every command the registry knows, in cheat-sheet order. A free function so the palette and menu
/// bar are built before a
/// session opens: opening one reads the keyring and can raise a Keychain prompt (ADR 0014).
#[uniffi::export]
pub fn commands() -> Vec<CommandSpecFfi> {
    registry::commands()
}

/// The ids `PostioKit.Intercepted` must hold, so the Swift copy can be checked against it (see
/// `registry::INTERCEPTED`).
#[uniffi::export]
pub fn intercepted_commands() -> Vec<String> {
    registry::INTERCEPTED
        .iter()
        .map(|id| id.to_string())
        .collect()
}

/// Which of Postio's interfaces the Mac app is, to the registry: Focus (ADR 0043).
pub const FRONTEND: postio_core::Frontend = postio_core::Frontend::Focus;

/// Which platform's design the boundary serves: Apple's, whatever host it was built on, so the
/// Linux gate (ADR 0019 Q7)
/// runs the composition the Mac runs. Never ask `Platform::host()`.
pub const PLATFORM: postio_config::paths::Platform = postio_config::paths::Platform::Apple;

uniffi::setup_scaffolding!();

/// Set aside the store at `store_path` (or the usual path) and start a fresh one there, carrying
/// its accounts across.
/// Blocks: call it off the main actor, then open again.
#[uniffi::export]
pub fn start_over(store_path: Option<String>) -> Result<StartedOverFfi, SessionError> {
    start_over_with(match store_path {
        Some(path) => SessionOptions::at(path),
        None => SessionOptions::at_default_path(),
    })
}

/// The words of the page a store that will not open shows, `postio_ui::focus_state`'s.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StoreRefusalWordsFfi {
    pub cant_open: String,
    pub try_again: String,
    pub from_another_build: String,
    pub start_over_sentence: String,
    pub under_another_key: String,
    pub start_over_under_another_key_sentence: String,
    pub start_over: String,
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
        under_another_key: words::STORE_UNDER_ANOTHER_KEY.to_owned(),
        start_over_under_another_key_sentence: words::START_OVER_UNDER_ANOTHER_KEY.to_owned(),
        start_over: words::START_A_FRESH_STORE.to_owned(),
        starting_over: words::STARTING_A_FRESH_STORE.to_owned(),
    }
}

#[uniffi::export]
pub fn started_over_words(set_aside: String) -> String {
    postio_ui::focus_state::started_over(&set_aside)
}

/// A placeholder export proving the scaffolding compiles under `forbid(unsafe_code)` and
/// `uniffi-bindgen` can read the metadata.
#[uniffi::export]
pub fn probe() -> String {
    "postio".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_is_callable_as_ordinary_rust() {
        // `#[uniffi::export]` adds a C-ABI shim beside the function; wrapping or moving it would
        // break in-process callers.
        assert_eq!(probe(), "postio");
    }
}
