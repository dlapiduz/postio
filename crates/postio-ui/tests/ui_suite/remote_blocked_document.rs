//! The document the reader hands its renderer, for a sender whose remote
//! content is blocked (#1926): no network URL under the hostile fixtures'
//! `vector.example.com` survives into it, in markup, styles or head. This is
//! the far end of `postio-body`'s `remote_vectors`, past `document_for`.

use postio_body::RemoteImages;
use postio_ui::reader::document::{self, Rendering};

#[test]
fn the_final_document_for_a_blocked_sender_names_no_network_resource() {
    let mut checked = 0;
    for fixture in postio_model::test_corpus::all()
        .iter()
        .filter(|fixture| fixture.name().starts_with("hostile-remote-"))
    {
        let body = postio_model::mime::parse(fixture.bytes()).body;
        assert!(
            body.html
                .as_deref()
                .is_some_and(|h| h.contains("vector.example.com")),
            "{} lost its references, so this test cannot fail",
            fixture.name()
        );
        let rendered =
            document::body_html_in(&body, RemoteImages::Blocked, Rendering::Original, None);
        let doc = document::document_for(
            &rendered.html,
            &rendered.styles,
            RemoteImages::Blocked,
            document::sheet_for(Rendering::Original, false),
        );
        assert!(
            !doc.to_ascii_lowercase().contains("vector.example.com"),
            "{}: the final document names a network resource:\n{doc}",
            fixture.name()
        );
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} fixtures were checked");
}
