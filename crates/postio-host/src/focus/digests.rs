//! Reads for the digest window: the held messages as list rows.

use postio_model::DeliveryId;
use postio_model::listing::{MessageSummary, StoreError};
use postio_storage::repository::DigestRepository;

use crate::Inner;

pub(crate) async fn held(
    inner: &Inner,
    messages: &[postio_model::MessageId],
) -> Result<Vec<(postio_model::MessageId, String, bool)>, StoreError> {
    let reader = inner.wiring.database.read().await?;
    Ok(DigestRepository::new(&reader).held(messages).await?)
}

pub(crate) async fn waiting(inner: &Inner, rules: &[String]) -> Result<Vec<u32>, StoreError> {
    let reader = inner.wiring.database.read().await?;
    let names: Vec<&str> = rules.iter().map(String::as_str).collect();
    Ok(DigestRepository::new(&reader).waiting(&names).await?)
}

pub(crate) async fn delivery_messages(
    inner: &Inner,
    delivery: DeliveryId,
) -> Result<Vec<MessageSummary>, StoreError> {
    let ids = {
        let reader = inner.wiring.database.read().await?;
        DigestRepository::new(&reader)
            .delivery_messages(delivery)
            .await?
    };
    let mut rows = inner.wiring.store.message_rows(ids).await?;
    rows.sort_by(|a, b| b.received_at.cmp(&a.received_at).then(b.id.cmp(&a.id)));
    Ok(rows)
}
