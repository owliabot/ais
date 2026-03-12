use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use ais_agent_host::events::HostEventServiceError;

#[derive(Debug)]
pub struct HttpApiError {
    status: StatusCode,
    body: HttpErrorBody,
}

#[derive(Debug, Serialize)]
struct HttpErrorBody {
    code: String,
    message: String,
}

impl HttpApiError {
    pub fn from_event_service_error(error: HostEventServiceError) -> Self {
        let status = status_for_event_service_error(&error.code);
        Self {
            status,
            body: HttpErrorBody {
                code: error.code,
                message: error.message,
            },
        }
    }
}

fn status_for_event_service_error(code: &str) -> StatusCode {
    if code.ends_with("_not_found") {
        StatusCode::NOT_FOUND
    } else if matches!(
        code,
        "session_run_mismatch"
            | "idempotency_conflict"
            | "version_conflict"
            | "stale_checkpoint_seq"
            | "stale_plan_epoch"
    ) || code.contains("conflict")
    {
        StatusCode::CONFLICT
    } else if matches!(
        code,
        "event_query_failed"
            | "repository_error"
            | "checkpoint_error"
            | "mission_error"
            | "event_archive_error"
            | "run_catalog_error"
            | "signer_archive_error"
            | "restore_error"
    ) || code.ends_with("_error")
    {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

impl IntoResponse for HttpApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}
