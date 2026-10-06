//! What Focus's surfaces ask of the host, and what it answers.
//!
//! `update` never does I/O: a surface that needs the store hands the loop an
//! [`Ask`] inside [`crate::app::Effect::Ask`], and the loop performs it and
//! feeds [`Answer`] back through [`crate::app::Input::Answer`]. Every answer
//! carries what it answers, so one that arrives after the person has moved
//! on is dropped.

use postio_client::Client;
use postio_client::protocol::FilteredRow;

/// A request of the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Each reason Filtered holds, with its count.
    FilteredTabs,
    /// A page of Filtered, newest first.
    FilteredPage {
        /// Which reading of the tab this belongs to.
        generation: u64,
        /// The reason, as the store spells it; `None` for All.
        reason: Option<String>,
        /// How many rows to skip.
        offset: u32,
    },
    /// How many messages a sweep of the inbox would file away.
    SweepPreview,
    /// What a digest's delivery holds and its summary, once one is written.
    Digest(postio_model::DeliveryId),
}

/// What the host answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Each reason and its count, or why not.
    FilteredTabs(Result<Vec<(String, u32)>, String>),
    /// A page of Filtered.
    FilteredPage {
        /// The generation it was asked in.
        generation: u64,
        /// Where it starts.
        offset: u32,
        /// Its rows, or why there are none.
        rows: Result<Vec<FilteredRow>, String>,
    },
    /// How many a sweep would move, or why not.
    SweepPreview(Result<u32, String>),
    /// A digest's messages, newest first, and its summary.
    Digest {
        /// Which delivery.
        delivery: postio_model::DeliveryId,
        /// Its messages, or why there are none.
        messages: Result<Vec<postio_model::listing::MessageSummary>, String>,
        /// Its summary: nothing until one is written.
        summary: Result<Option<postio_model::summary::DigestSummary>, String>,
    },
}

impl Ask {
    /// Ask `client`.
    pub async fn perform(self, client: &Client) -> Answer {
        let said = |error: postio_model::listing::StoreError| error.message().to_owned();
        match self {
            Ask::FilteredTabs => Answer::FilteredTabs(client.filtered_tabs().await.map_err(said)),
            Ask::FilteredPage {
                generation,
                reason,
                offset,
            } => Answer::FilteredPage {
                generation,
                offset,
                rows: client
                    .filtered(reason, offset, postio_ui::filtered::PAGE)
                    .await
                    .map_err(said),
            },
            Ask::SweepPreview => Answer::SweepPreview(client.sweep_preview().await.map_err(said)),
            Ask::Digest(delivery) => Answer::Digest {
                delivery,
                messages: client.delivery_messages(delivery).await.map_err(said),
                summary: client.digest_summary(delivery).await.map_err(said),
            },
        }
    }
}
