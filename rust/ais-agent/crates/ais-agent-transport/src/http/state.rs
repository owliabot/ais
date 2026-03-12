use std::sync::Arc;

use ais_agent_host::{control::HostCommandService, events::HostRunEventService};
use tokio::sync::Mutex;

#[derive(Debug)]
pub struct HttpServiceState<S> {
    pub service: Arc<Mutex<S>>,
}

impl<S> Clone for HttpServiceState<S> {
    fn clone(&self) -> Self {
        Self {
            service: Arc::clone(&self.service),
        }
    }
}

impl<S> HttpServiceState<S>
where
    S: HostCommandService + HostRunEventService,
{
    pub fn new(service: S) -> Self {
        Self {
            service: Arc::new(Mutex::new(service)),
        }
    }
}
