//! The reader's remote images, fetched through the shared glue (spec 006
//! FR-025; specs/007-postio-focus T022): the reader asks by the document's
//! own spelling, the app's fetcher answers by parsed URL on its runtime, and
//! what arrives is handed back on the main loop under the spelling it was
//! asked by. A failed image is absent, not an empty picture.

use std::cell::RefCell;
use std::rc::Rc;

use crate::support::until;

/// What came back to the main loop, once it has.
type Arrived = Rc<RefCell<Option<Vec<(String, Vec<u8>)>>>>;

pub fn fetched_images_come_back_under_the_documents_spelling() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("a runtime");
    let fetch = postio_widgets::present::reading::remote_fetch(
        runtime.handle().clone(),
        |urls: Vec<url::Url>| async move {
            urls.into_iter()
                .map(|url| {
                    let bytes = url.path().ends_with("map.png").then(|| b"PNG".to_vec());
                    (url, bytes)
                })
                .collect()
        },
    );
    let arrived: Arrived = Rc::default();
    fetch(
        vec![
            "https://Images.invalid/map.png".to_owned(),
            "https://images.invalid/gone.png".to_owned(),
            "not a url".to_owned(),
        ],
        Box::new({
            let arrived = arrived.clone();
            move |images| {
                arrived.replace(Some(images));
            }
        }),
    );
    assert!(
        until(|| arrived.borrow().is_some()),
        "nothing came back to the main loop"
    );
    assert_eq!(
        arrived.borrow().clone().expect("an answer"),
        [("https://Images.invalid/map.png".to_owned(), b"PNG".to_vec())],
        "the image did not come back under the spelling the document asked by, \
         or a failed one came back at all"
    );
}
