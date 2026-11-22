//! The reader's remote images, fetched for it. This crate reaches no network: [`remote_fetch`]
//! joins the reader to the
//! fetcher the composition root owns, on that root's runtime.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use gtk::glib;

use crate::reader::view::RemoteArrived;

/// `None` for a URL that failed or was refused.
pub type Fetched = Vec<(url::Url, Option<Vec<u8>>)>;

/// The reader looks images up by the document's own spelling of each URL, so the answer is mapped
/// back to it. A URL that
/// does not parse is never asked for.
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
