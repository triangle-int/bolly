use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};

use crate::{app::state::AppState, domain::upload::UploadMeta, services::uploads};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/{instance_slug}/uploads",
            get(list_uploads).post(upload_file),
        )
        .layer(DefaultBodyLimit::max(500 * 1024 * 1024)) // 500 MB
        .route(
            "/api/instances/{instance_slug}/uploads/{upload_id}",
            get(get_upload_meta).delete(delete_upload),
        )
        .route(
            "/api/instances/{instance_slug}/uploads/{upload_id}/file",
            get(serve_file),
        )
}

/// Public file-serving route (no auth middleware) — uses ?token= query param.
/// Used by LLM providers (OpenRouter) to fetch uploaded files via URL.
pub fn public_router() -> Router<AppState> {
    Router::new().route(
        "/public/files/{instance_slug}/{upload_id}",
        get(serve_file_public),
    )
}

async fn upload_file(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<UploadMeta>, (StatusCode, String)> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name != "file" {
            continue;
        }

        let file_name = field.file_name().unwrap_or("unnamed").to_string();

        let bytes = field
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("failed to read file: {e}")))?;

        let meta = uploads::save_upload(&state.workspace_dir, &instance_slug, &file_name, &bytes)
            .map_err(|e| {
            let status = match e.kind() {
                std::io::ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, e.to_string())
        })?;

        return Ok(Json(meta));
    }

    Err((StatusCode::BAD_REQUEST, "no file field in upload".into()))
}

async fn list_uploads(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
) -> Result<Json<Vec<UploadMeta>>, (StatusCode, String)> {
    let items = uploads::list_uploads(&state.workspace_dir, &instance_slug)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(items))
}

async fn get_upload_meta(
    State(state): State<AppState>,
    Path((instance_slug, upload_id)): Path<(String, String)>,
) -> Result<Json<UploadMeta>, StatusCode> {
    match uploads::get_upload(&state.workspace_dir, &instance_slug, &upload_id) {
        Ok(Some(meta)) => Ok(Json(meta)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn serve_file(
    State(state): State<AppState>,
    Path((instance_slug, upload_id)): Path<(String, String)>,
) -> Result<Response, StatusCode> {
    serve_file_inner(&state, &instance_slug, &upload_id).await
}

#[derive(serde::Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

async fn serve_file_public(
    State(state): State<AppState>,
    Path((instance_slug, upload_id)): Path<(String, String)>,
    Query(query): Query<TokenQuery>,
) -> Result<Response, StatusCode> {
    // Verify token matches auth_token
    let expected = state.config.read().await.auth_token.clone();
    if expected.is_empty() || query.token.as_deref() != Some(&expected) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    serve_file_inner(&state, &instance_slug, &upload_id).await
}

async fn serve_file_inner(
    state: &AppState,
    instance_slug: &str,
    upload_id: &str,
) -> Result<Response, StatusCode> {
    let meta = uploads::get_upload(&state.workspace_dir, instance_slug, upload_id)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let file_path = uploads::get_upload_file_path(&state.workspace_dir, instance_slug, upload_id)
        .ok_or(StatusCode::NOT_FOUND)?;

    let bytes = tokio::fs::read(&file_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let is_media = meta.mime_type.starts_with("audio/")
        || meta.mime_type.starts_with("video/")
        || meta.mime_type.starts_with("image/");
    let disposition = if is_media { "inline" } else { "attachment" };
    let content_disposition = format!(
        "{disposition}; filename=\"{}\"",
        meta.original_name.replace('"', "_")
    );

    Response::builder()
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&meta.mime_type)
                .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
        )
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&content_disposition)
                .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
        )
        .header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        )
        .body(Body::from(bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn delete_upload(
    State(state): State<AppState>,
    Path((instance_slug, upload_id)): Path<(String, String)>,
) -> StatusCode {
    match uploads::delete_upload(&state.workspace_dir, &instance_slug, &upload_id) {
        Ok(true) => StatusCode::OK,
        Ok(false) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
