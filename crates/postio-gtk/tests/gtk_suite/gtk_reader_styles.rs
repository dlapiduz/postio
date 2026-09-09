//! One sender's stylesheet against another sender's message (#1326).

/// Can one message's `<style>` block restyle another message? (#1326)
///
/// The question ADR 0032 turns on. Its stated reason for putting a whole
/// conversation in one document was that the sanitizer had deleted every
/// sender's CSS, so messages could not contaminate each other. #1325 admitted
/// inline styling and #1326 admits `<style>` blocks, so that reason is gone
/// and the containment now rests on `postio_body::styles` rewriting every
/// selector under its own message's container.
///
/// That is a claim about a *parser*, and the parser's own tests can only
/// check the text it emits. Whether the text means what it is supposed to
/// mean is a question for the engine, so this asks the engine.
///
/// Cascade, not layout: unlike `one_senders_styling_cannot_reach_another_message`
/// this needs no boxes, so it runs everywhere including CI (#1307).
pub fn one_senders_stylesheet_cannot_restyle_another_message() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    const RED: &str = "rgb(1, 2, 3)";
    // Two rules a sender would plausibly write, and one they would not:
    // `p` reaches the other message's paragraphs, `body` is how a great deal
    // of real mail sets its type, and `.postio-blocked` is the notice saying
    // this very message's images were held back.
    let hostile = format!(
        "<style>p {{ color: {RED} }} body {{ color: {RED} }} \
         .postio-blocked {{ display: none }}</style><p>first</p>"
    );

    let sanitized = postio_body::sanitize::sanitize_body_in(
        &hostile,
        postio_body::RemoteImages::Blocked,
        Some("1"),
    );
    let innocent = postio_body::sanitize::sanitize_body_in(
        "<p>second</p>",
        postio_body::RemoteImages::Blocked,
        Some("2"),
    );

    fn entry<'a>(
        scope: &'a str,
        body: &'a str,
        styles: &'a str,
    ) -> postio_ui::reader::thread::Entry<'a> {
        postio_ui::reader::thread::Entry {
            scope,
            sender: "Ada Lovelace",
            address: "ada@example.com",
            when: "09:14",
            preview: "the first line of it",
            expanded: true,
            latest: false,
            blocked: 0,
            body,
            styles,
        }
    }
    // Selected through the containers Postio itself writes: ammonia drops a
    // sender's `id`, so a probe element cannot carry one of its own.
    let mine = postio_body::sanitize::message_selector(Some("1"));
    let theirs = postio_body::sanitize::message_selector(Some("2"));

    let document = postio_ui::reader::thread::conversation_document(
        &[
            entry("1", &sanitized.html, &sanitized.styles),
            entry("2", &innocent.html, &innocent.styles),
        ],
        postio_body::RemoteImages::Blocked,
        postio_ui::reader::document::Sheet::Theme,
    );

    // **The control, first.** Without it every assertion below passes when
    // the stylesheet is simply dropped -- which is what the code did before
    // #1326 and is not what it is supposed to do now. FR-019 says a message
    // renders as its sender built it.
    assert_eq!(
        crate::webkit_probe::computed(&document, &format!("{} p", mine), "color"),
        RED,
        "the sender's own rule did not reach their own message, so the \
         assertions below prove nothing: a dropped stylesheet contaminates \
         no one either"
    );

    assert_ne!(
        crate::webkit_probe::computed(&document, &format!("{} p", theirs), "color"),
        RED,
        "one sender's `<style>` restyled another sender's message. ADR 0032 \
         puts them in one document; `postio_body::styles` is the only thing \
         keeping them apart"
    );
}
