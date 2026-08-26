//! The composer's editing surface: the one `WebView` where JavaScript runs.
//!
//! ADR 0003's licence, restated at the place it applies: script that arrived
//! in a message never executes anywhere, and Postio's own bundled script is
//! not message content. This view exists to run *that* script — the editing
//! bridge — over a `contenteditable` document, and nothing else changes: the
//! reader keeps JavaScript off, and this profile keeps every other door the
//! reader closes closed too. `enable_javascript_markup` stays **off**, which
//! is the setting that makes the distinction mechanical rather than
//! disciplinary — a `<script>` tag inside edited or pasted content is inert
//! markup here, while the host's own injected script runs.
//!
//! Network is closed by construction, the same three ways the reader closes
//! it: an ephemeral [`NetworkSession`], a [`WebContext`] whose only
//! registered scheme resolves `postio-cid:` from the local blob store, and a
//! CSP on the editing shell that names no remote origin. The dialect the
//! surface emits is pinned by `tests/gtk_editable_dialect.rs`; the paragraph
//! separator and `styleWithCSS` settings that dialect depends on are applied
//! here, by an injected script, so no later caller can forget them.
//!
//! [`NetworkSession`]: webkit6::NetworkSession
//! [`WebContext`]: webkit6::WebContext

use std::rc::Rc;

use webkit6::prelude::*;

use crate::reader::scheme::{self, BlobSource};

/// A fixed, non-`http(s)` base for the editing shell, so edited content is
/// never same-origin with anything real — the same reasoning as the
/// reader's `postio-reader:///`.
pub const EDITOR_BASE_URI: &str = "postio-editor:///";

/// The CSP the editing shell carries: no remote origin can be named, styles
/// stay inline (the shell's own), and images resolve only through the local
/// blob scheme.
const EDITOR_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src postio-cid:";

/// The profile settings the dialect contract depends on, applied the moment
/// the document exists so no gesture can run before them.
const PROFILE_SCRIPT: &str = "document.execCommand('defaultParagraphSeparator', false, 'p'); \
     document.execCommand('styleWithCSS', false, 'false');";

/// Build the editing view: JavaScript on for the host, everything else the
/// reader's lockdown list closes, closed.
///
/// `source` resolves `postio-cid:` references — pasted inline images, once
/// #341 lands them in the blob store. The view arrives empty; [`seed`] loads
/// the editing shell.
pub fn editing_view(source: Rc<dyn BlobSource>) -> webkit6::WebView {
    let network_session = webkit6::NetworkSession::new_ephemeral();
    network_session.set_persistent_credential_storage_enabled(false);

    let context = webkit6::WebContext::new();
    scheme::register(&context, source);

    let content = webkit6::UserContentManager::new();
    content.add_script(&webkit6::UserScript::new(
        PROFILE_SCRIPT,
        webkit6::UserContentInjectedFrames::TopFrame,
        webkit6::UserScriptInjectionTime::End,
        &[],
        &[],
    ));

    let view = webkit6::WebView::builder()
        .web_context(&context)
        .network_session(&network_session)
        .user_content_manager(&content)
        .settings(&editing_settings())
        .hexpand(true)
        .vexpand(true)
        .build();
    view.add_css_class("postio-editor-view");
    view.set_accessible_role(gtk::AccessibleRole::TextBox);
    view.connect_decide_policy(handle_decide_policy);
    view
}

/// Load the editing shell around `inner_html`, which must already be
/// canonical-subset markup (a `Document`'s `to_html`, or empty).
///
/// The shell is Postio's own markup, not message content — quoted material
/// goes through `postio_body::parse` *before* it can appear here, which is
/// what makes running script beside it acceptable at all (ADR 0003,
/// hardening requirement 2).
pub fn seed(view: &webkit6::WebView, inner_html: &str) {
    let shell = format!(
        "<!doctype html><html><head>\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{EDITOR_CSP}\">\
         </head><body contenteditable=\"true\">{inner_html}</body></html>"
    );
    view.load_html(&shell, Some(EDITOR_BASE_URI));
}

/// The reader's lockdown list with exactly one line changed.
///
/// Deliberately not shared with `reader::view::hardened_settings`: the two
/// profiles must be *diffable at a glance*, and a shared function with a
/// flag would hide the one difference that matters inside a parameter.
fn editing_settings() -> webkit6::Settings {
    let settings = webkit6::Settings::new();
    // The one difference: the host's script runs here. Markup-borne script
    // still does not — that line is identical to the reader's on purpose.
    settings.set_enable_javascript(true);
    settings.set_enable_javascript_markup(false);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_javascript_can_access_clipboard(false);
    settings.set_enable_html5_database(false);
    settings.set_enable_html5_local_storage(false);
    settings.set_enable_page_cache(false);
    settings.set_enable_media(false);
    settings.set_enable_media_stream(false);
    settings.set_enable_mediasource(false);
    settings.set_enable_encrypted_media(false);
    settings.set_enable_webrtc(false);
    settings.set_enable_webgl(false);
    settings.set_enable_webaudio(false);
    settings.set_enable_fullscreen(false);
    settings.set_enable_developer_extras(cfg!(debug_assertions));
    settings
}

/// Nothing navigates an editor.
///
/// The reader lets a user's own click leave for the browser; here a click on
/// a link is an editing gesture — the caret moves into the link text — so
/// every link-clicked navigation is refused outright, and everything else
/// (the initial `load_html`) proceeds as WebKit intends.
fn handle_decide_policy(
    _view: &webkit6::WebView,
    decision: &webkit6::PolicyDecision,
    kind: webkit6::PolicyDecisionType,
) -> bool {
    if kind != webkit6::PolicyDecisionType::NavigationAction {
        return false;
    }
    let Some(mut action) = decision
        .downcast_ref::<webkit6::NavigationPolicyDecision>()
        .and_then(|decision| decision.navigation_action())
    else {
        return false;
    };
    if action.navigation_type() == webkit6::NavigationType::LinkClicked {
        decision.ignore();
        return true;
    }
    false
}
