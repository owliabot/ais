use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};

use crate::events::{HostRunEventBatch, HostRunEventQuery};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostEventServiceError {
    pub code: String,
    pub message: String,
}

pub trait HostRunEventService {
    fn list_events(
        &self,
        query: HostRunEventQuery,
    ) -> Pin<Box<dyn Future<Output = Result<HostRunEventBatch, HostEventServiceError>> + Send + '_>>;
}
