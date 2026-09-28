//! The person's own model, as Focus's tasks reach it (spec 007 milestone 2,
//! FR-165 to FR-170).
//!
//! **Nothing connects unless `[focus.model]` names a model** on this
//! computer and the feature's switch is on (FR-166, SC-016). Reading the
//! section connects to nothing and looks nothing up; a client exists only
//! once a feature is asked for, and it connects only when it is asked
//! something.
//!
//! One client per endpoint and model, kept across batches and features, so a
//! runtime that is not running is left alone for a while by all of them
//! (`postio_ai::RETRY_AFTER`), and every call is recorded in the egress log.

use std::sync::{Arc, Mutex};

use postio_ai::{Client, NeedsActionModel, Transport};
use postio_config::{FocusConfig, ModelEndpoint, ModelFeature};
use postio_model::egress::EgressSink;

/// Where Focus's tasks get the person's model from.
pub(crate) struct Models {
    /// How to reach it: a test's fake, or this computer's own sockets.
    transport: Option<Arc<dyn Transport>>,
    /// Where every call is recorded.
    egress: Arc<dyn EgressSink>,
    /// The client for the endpoint and model last named.
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

    /// The client for `feature`, as `config` says: `None` unless
    /// `[focus.model]` is there, usable, and has `feature`'s switch on.
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

    /// The model that answers the needs-action question, when there is one.
    pub(crate) fn needs_action(&self, config: &FocusConfig) -> Option<Arc<NeedsActionModel>> {
        self.client(config, ModelFeature::NeedsAction)
            .map(|client| Arc::new(NeedsActionModel::new(client)))
    }
}
