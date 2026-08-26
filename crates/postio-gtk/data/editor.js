/* The composer's editing bridge — the one script Postio runs in a WebView.
 *
 * ADR 0003: this is the host's own code, shipped in the GResource bundle,
 * never message content; the profile it runs under keeps markup-borne
 * script inert (enable_javascript_markup off) and the shell CSP names no
 * remote origin. Its whole job is two things:
 *
 *   1. Pin the dialect the contract test (gtk_editable_dialect.rs) proves:
 *      <p> paragraphs and element-form bold/italic, set before any gesture
 *      can run.
 *   2. Report every edit to the host as the DOM's innerHTML, which the host
 *      parses back into the canonical Document — the DOM is a working copy,
 *      never the record (ADR 0004 Q3).
 *
 * No timers, no network, no state beyond the document it edits.
 */
document.execCommand('defaultParagraphSeparator', false, 'p');
document.execCommand('styleWithCSS', false, 'false');

document.addEventListener('input', () => {
    window.webkit.messageHandlers.postioEdited.postMessage(
        document.body.innerHTML
    );
});
