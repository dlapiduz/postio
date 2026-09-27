//! The reader's remote images, fetched for it (spec 006 FR-025, T137).
//!
//! The reader names the URLs a document shows as images, and only for a
//! sender the user allowed or a message they chose to show once. It fetches
//! nothing itself: this crate reaches no network. [`remote_fetch`] joins the
//! reader to whatever fetcher the app's composition root owns, on that root's
//! runtime, and hands what arrived back on the main loop.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use gtk::glib;

use crate::reader::view::RemoteArrived;

/// What a batch fetch answered for each URL: its bytes, or `None` for a URL
/// that failed or was refused.
pub type Fetched = Vec<(url::Url, Option<Vec<u8>>)>;

/// The reader's remote-image fetch (`Reader::set_remote_fetch`), over
/// `fetch_all` run on `runtime`.
///
/// The document's own spelling of each URL is what the reader looks its
/// images up by, and a fetch reports the parsed URL back, so the answer is
/// mapped back to the spelling it was asked by. A URL that does not parse is
/// never asked for; one that failed is simply absent from what arrives.
pub fn remote_fetch<F, Fut>(
    runtime: tokio::runtime::Handle,
    fetch_all: F,
) -> impl Fn(Vec<String>, RemoteArrived) + 'static
where
    F: Fn(Vec<url::Url>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Fetched> + Send + 'static,
{
    let fetch_all = Arc::new(fetch_all);
    move |urls, done| {
        let spelled: HashMap<String, String> = urls
            .iter()
            .filter_map(|raw| Some((url::Url::parse(raw).ok()?.to_string(), raw.clone())))
            .collect();
        let parsed: Vec<url::Url> = spelled
            .keys()
            .filter_map(|url| url::Url::parse(url).ok())
            .collect();
        let (sender, receiver) = async_channel::bounded(1);
        let fetch_all = Arc::clone(&fetch_all);
        runtime.spawn(async move {
            let _ = sender.send(fetch_all(parsed).await).await;
        });
        glib::spawn_future_local(async move {
            let Ok(fetched) = receiver.recv().await else {
                return;
            };
            let arrived = fetched
                .into_iter()
                .filter_map(|(url, bytes)| Some((spelled.get(url.as_str())?.clone(), bytes?)))
                .collect();
            done(arrived);
        });
    }
}
