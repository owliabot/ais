use ais_agent_host::{
    control::{HostCommandError, HostCommandOutcome, HostCommandResponse, HostCommandService},
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    session::HostedRunCommand,
};

#[derive(Debug, Clone)]
pub struct UnavailableHostService {
    code: String,
    message: String,
    event_message: String,
}

impl UnavailableHostService {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        let code = code.into();
        let message = message.into();
        let event_message = message.clone();
        Self {
            code,
            message,
            event_message,
        }
    }
}

impl Default for UnavailableHostService {
    fn default() -> Self {
        Self::new(
            "runtime_not_wired",
            "ais-agent CLI transport shell is available, but no runtime service is wired yet",
        )
    }
}

impl HostCommandService for UnavailableHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Error(HostCommandError {
                    code: self.code.clone(),
                    message: self.message.clone(),
                }),
                events: Vec::new(),
            }
        })
    }
}

impl HostRunEventService for UnavailableHostService {
    fn list_events(
        &self,
        _query: HostRunEventQuery,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<HostRunEventBatch, HostEventServiceError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            Err(HostEventServiceError {
                code: self.code.clone(),
                message: self.event_message.clone(),
            })
        })
    }
}
