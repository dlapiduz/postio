//! The person's own model, as Focus's tasks reach it. Nothing connects unless `[focus.model]` names
//! a model and the feature's switch is on.
//! One client per endpoint and model, kept across batches, so a runtime that is not running is left
//! alone (`postio_ai::RETRY_AFTER`); every call is in the egress log.

use std::sync::{Arc, Mutex};

use postio_ai::{Client, NeedsActionModel, Transport};
use postio_config::{FocusConfig, ModelEndpoint, ModelFeature};
use postio_model::egress::EgressSink;

pub(crate) struct Models {
    transport: Option<Arc<dyn Transport>>,
    egress: Arc<dyn EgressSink>,
    client: Mutex<Option<(ModelEndpoint, String, Arc<Client>)>>,
}

impl std::fmt::Debug for Models {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Models").finish_non_exhaustive()
    }
}

impl Models {
    pub(crate) fn new(transport: Option<Arc<dyn Transport>>, egress: Arc<dyn EgressSink>) -> Self {
        Self {
            transport,
            egress,
            client: Mutex::new(None),
        }
    }

    pub(crate) fn client(
        &self,
        config: &FocusConfig,
        feature: ModelFeature,
    ) -> Option<Arc<Client>> {
        let (endpoint, model) = config.model_for(feature)?;
        let mut kept = self.client.lock().expect("never poisoned");
        if let Some((kept_endpoint, kept_model, client)) = &*kept
            && *kept_endpoint == endpoint
            && kept_model == model
        {
            return Some(Arc::clone(client));
        }
        let mut client = Client::new(endpoint.clone(), model).with_egress(Arc::clone(&self.egress));
        if let Some(transport) = &self.transport {
            client = client.with_transport(Arc::clone(transport));
        }
        let client = Arc::new(client);
        *kept = Some((endpoint, model.to_owned(), Arc::clone(&client)));
        Some(client)
    }

    pub(crate) fn needs_action(&self, config: &FocusConfig) -> Option<Arc<NeedsActionModel>> {
        self.client(config, ModelFeature::NeedsAction)
            .map(|client| Arc::new(NeedsActionModel::new(client)))
    }
}
