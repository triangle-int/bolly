use crate::{
    app::state::AppState,
    services::resource_capability::{CapabilityAudience, CapabilityResource},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, OriginalUri, Path, State},
    http::{Method, StatusCode},
    response::Response,
    routing::{get, post},
};

pub(crate) fn issuance_router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/{slug}/resource-capabilities/memory",
            post(issue_memory),
        )
        .route(
            "/api/instances/{slug}/resource-capabilities/files",
            post(issue_file),
        )
        .route(
            "/api/native-relay/instances/{slug}/resource-capabilities/memory",
            post(issue_native_memory),
        )
        .route(
            "/api/native-relay/instances/{slug}/resource-capabilities/files",
            post(issue_native_file),
        )
        .layer(DefaultBodyLimit::max(2048))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryInput {
    path: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FileInput {
    id: String,
}

async fn issue_memory(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(input): Json<MemoryInput>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    issue(
        &state,
        &slug,
        CapabilityResource::memory(input.path).map_err(|_| StatusCode::BAD_REQUEST)?,
    )
}
async fn issue_file(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(input): Json<FileInput>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    issue(
        &state,
        &slug,
        CapabilityResource::uploaded_file(input.id).map_err(|_| StatusCode::BAD_REQUEST)?,
    )
}
async fn issue_native_memory(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(input): Json<MemoryInput>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let resource = CapabilityResource::memory(input.path).map_err(|_| StatusCode::BAD_REQUEST)?;
    let url = state
        .resources
        .url("", &slug, resource, CapabilityAudience::NativeRelay)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(
        serde_json::json!({ "url": url, "refresh_after_seconds": 600 }),
    ))
}
async fn issue_native_file(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Json(input): Json<FileInput>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let resource =
        CapabilityResource::uploaded_file(input.id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let url = state
        .resources
        .url("", &slug, resource, CapabilityAudience::NativeRelay)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(
        serde_json::json!({ "url": url, "refresh_after_seconds": 600 }),
    ))
}
fn issue(
    state: &AppState,
    slug: &str,
    resource: CapabilityResource,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let url = state
        .resources
        .url("", slug, resource, CapabilityAudience::Browser)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Json(
        serde_json::json!({ "url": url, "refresh_after_seconds": 600 }),
    ))
}

// Audiences are selected by trusted handlers, never request parameters.
macro_rules! handler {
    ($name:ident, $audience:ident, $resource:ident, $reader:ident, $module:ident) => {
        async fn $name(
            State(state): State<AppState>,
            Path((slug, path)): Path<(String, String)>,
            OriginalUri(uri): OriginalUri,
            method: Method,
        ) -> Result<Response, StatusCode> {
            let resource = CapabilityResource::$resource(path.clone())
                .map_err(|_| StatusCode::UNAUTHORIZED)?;
            state
                .resources
                .verify(
                    &slug,
                    resource,
                    CapabilityAudience::$audience,
                    &uri,
                    method.as_str(),
                )
                .map_err(|_| StatusCode::UNAUTHORIZED)?;
            super::$module::$reader(&state, &slug, &path).await
        }
    };
}
handler!(
    browser_file,
    Browser,
    uploaded_file,
    serve_file_inner,
    uploads
);
handler!(
    model_file,
    ModelProvider,
    uploaded_file,
    serve_file_inner,
    uploads
);
handler!(
    native_file,
    NativeRelay,
    uploaded_file,
    serve_file_inner,
    uploads
);
handler!(
    browser_memory,
    Browser,
    memory,
    serve_memory_file_inner,
    instances
);
handler!(
    model_memory,
    ModelProvider,
    memory,
    serve_memory_file_inner,
    instances
);
handler!(
    native_memory,
    NativeRelay,
    memory,
    serve_memory_file_inner,
    instances
);

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/resources/browser/files/{slug}/{id}", get(browser_file))
        .route(
            "/resources/model-provider/files/{slug}/{id}",
            get(model_file),
        )
        .route(
            "/resources/native-relay/files/{slug}/{id}",
            get(native_file),
        )
        .route(
            "/resources/browser/memory/{slug}/{*path}",
            get(browser_memory),
        )
        .route(
            "/resources/model-provider/memory/{slug}/{*path}",
            get(model_memory),
        )
        .route(
            "/resources/native-relay/memory/{slug}/{*path}",
            get(native_memory),
        )
}
