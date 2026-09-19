//! Activity receipts and policy for the one proactive loop (#92).

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;

use crate::{
    app::state::AppState,
    domain::proactive::{ProactivePolicy, ProactiveRun},
    services::proactive::Admission,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/{instance_slug}/activity",
            get(list_activity),
        )
        .route(
            "/api/instances/{instance_slug}/activity/{run_id}",
            get(get_activity),
        )
        .route(
            "/api/instances/{instance_slug}/activity/{run_id}/cancel",
            post(cancel_activity),
        )
        .route(
            "/api/instances/{instance_slug}/activity/{run_id}/retry",
            post(retry_activity),
        )
        .route(
            "/api/instances/{instance_slug}/proactive",
            get(get_policy).put(set_policy),
        )
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

async fn list_activity(
    State(state): State<AppState>,
    Path(_instance_slug): Path<String>,
    Query(query): Query<ListQuery>,
) -> Json<Vec<ProactiveRun>> {
    Json(state.proactive.list(query.limit.clamp(1, 500)))
}

async fn get_activity(
    State(state): State<AppState>,
    Path((_instance_slug, run_id)): Path<(String, String)>,
) -> Result<Json<ProactiveRun>, StatusCode> {
    state
        .proactive
        .get(&run_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn cancel_activity(
    State(state): State<AppState>,
    Path((_instance_slug, run_id)): Path<(String, String)>,
) -> StatusCode {
    if state.proactive.get(&run_id).is_none() {
        return StatusCode::NOT_FOUND;
    }
    if state.proactive.cancel(&run_id) {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    }
}

async fn retry_activity(
    State(state): State<AppState>,
    Path((_instance_slug, run_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<ProactiveRun>), (StatusCode, String)> {
    if state.proactive.get(&run_id).is_none() {
        return Err((StatusCode::NOT_FOUND, format!("unknown run {run_id}")));
    }
    let now = chrono::Utc::now().timestamp();
    match state.proactive.retry(&run_id, now) {
        Ok(Admission::Admitted(handle)) => {
            // The retry is admitted as a pending run; the owning trigger's worker
            // (#93 narrows this to the companion check-in) executes it. Until
            // then it is visible and cancellable like any other run.
            let run = state
                .proactive
                .get(handle.id())
                .ok_or((StatusCode::INTERNAL_SERVER_ERROR, "run vanished".into()))?;
            std::mem::forget(handle);
            Ok((StatusCode::ACCEPTED, Json(run)))
        }
        Ok(Admission::Skipped(run)) => Ok((StatusCode::ACCEPTED, Json(run))),
        Err(message) => Err((StatusCode::CONFLICT, message)),
    }
}

async fn get_policy(
    State(state): State<AppState>,
    Path(_instance_slug): Path<String>,
) -> Json<ProactivePolicy> {
    Json(state.proactive.policy())
}

async fn set_policy(
    State(state): State<AppState>,
    Path(_instance_slug): Path<String>,
    Json(policy): Json<ProactivePolicy>,
) -> Result<StatusCode, (StatusCode, String)> {
    if let Some(quiet) = policy.quiet_hours
        && (quiet.start_hour > 23 || quiet.end_hour > 23)
    {
        return Err((StatusCode::BAD_REQUEST, "quiet hours must be 0-23".into()));
    }
    if policy.retention_max == 0 || policy.retention_days == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "retention must keep at least one record for one day".into(),
        ));
    }
    state
        .proactive
        .set_policy(&policy)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(StatusCode::OK)
}
