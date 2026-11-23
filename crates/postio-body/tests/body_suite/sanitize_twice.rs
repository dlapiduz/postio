use postio_body::sanitize::{RemoteImages, sanitize_body_in};
use postio_model::test_corpus;

#[test]
fn a_second_pass_over_any_corpus_body_changes_nothing() {
    let mut changed = Vec::new();
    for fixture in test_corpus::all() {
        let message = fixture.parse();
        let Some(html) = message.body.html.as_deref() else {
            continue;
        };
        for scope in [None, Some("7")] {
            let once = sanitize_body_in(html, RemoteImages::Blocked, scope);
            let twice = sanitize_body_in(&once.html, RemoteImages::Blocked, scope);
            if twice.html != once.html {
                changed.push(format!("{} (scope {scope:?})", fixture.name()));
            }
        }
    }
    assert!(changed.is_empty(), "a second pass changed {changed:?}");
}
