use ais_agent_host::{
    control::{HostCommandError, HostCommandOutcome, HostCommandResponse, HostCommandService},
    events::{HostEventServiceError, HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    session::HostedRunCommand,
};

#[derive(Debug, Default)]
pub struct UnavailableHostService {
    _private: (),
}

impl HostCommandService for UnavailableHostService {
    fn handle(
        &mut self,
        _command: HostedRunCommand,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = HostCommandOutcome> + Send + '_>> {
        Box::pin(async move {
            HostCommandOutcome {
                response: HostCommandResponse::Error(HostCommandError {
                    code: "runtime_not_wired".to_owned(),
                    message: "ais-agent CLI transport shell is available, but no runtime service is wired yet"
                        .to_owned(),
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
                code: "runtime_not_wired".to_owned(),
                message: "ais-agent CLI transport shell is available, but no runtime event service is wired yet"
                    .to_owned(),
            })
        })
    }
}
