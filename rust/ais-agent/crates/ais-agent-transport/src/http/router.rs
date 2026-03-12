use axum::{
    extract::{Path, Query, State},
    routing::{get, post, MethodRouter},
    Json, Router,
};
use serde::Deserialize;

use ais_agent_control::ids::RunId;
use ais_agent_host::{
    control::{HostCommandOutcome, HostCommandService},
    events::{HostRunEventBatch, HostRunEventQuery, HostRunEventService},
    session::HostedRunCommand,
};

use crate::http::{error::HttpApiError, state::HttpServiceState};

pub fn build_http_router<S>(service: S) -> Router
where
    S: HostCommandService + HostRunEventService + Send + 'static,
{
    Router::new()
        .merge(command_routes::<S>())
        .merge(event_routes::<S>())
        .with_state(HttpServiceState::new(service))
}

fn command_routes<S>() -> Router<HttpServiceState<S>>
where
    S: HostCommandService + HostRunEventService + Send + 'static,
{
    route("/commands", post(handle_command::<S>))
}

fn event_routes<S>() -> Router<HttpServiceState<S>>
where
    S: HostCommandService + HostRunEventService + Send + 'static,
{
    route("/runs/{run_id}/events", get(handle_event_poll::<S>))
}

fn route<S>(
    path: &str,
    method_router: MethodRouter<HttpServiceState<S>>,
) -> Router<HttpServiceState<S>>
where
    S: HostCommandService + HostRunEventService + Send + 'static,
{
    Router::new().route(path, method_router)
}

async fn handle_command<S>(
    State(state): State<HttpServiceState<S>>,
    Json(command): Json<HostedRunCommand>,
) -> Json<HostCommandOutcome>
where
    S: HostCommandService + HostRunEventService + Send + 'static,
{
    let mut service = state.service.lock().await;
    let response = service.handle(command).await;

    Json(response)
}

#[derive(Debug, Deserialize)]
struct EventPollParams {
    after_event_seq: Option<u64>,
    limit: Option<usize>,
}

async fn handle_event_poll<S>(
    State(state): State<HttpServiceState<S>>,
    Path(run_id): Path<String>,
    Query(params): Query<EventPollParams>,
) -> Result<Json<HostRunEventBatch>, HttpApiError>
where
    S: HostCommandService + HostRunEventService + Send + 'static,
{
    let service = state.service.lock().await;
    let query = HostRunEventQuery {
        run_id: RunId(run_id),
        after_event_seq: params.after_event_seq,
        limit: params.limit,
    };

    match service.list_events(query).await {
        Ok(batch) => Ok(Json(batch)),
        Err(error) => Err(HttpApiError::from_event_service_error(error)),
    }
}
