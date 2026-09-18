use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    routing::post,
};
use serde::Serialize;

use crate::app::state::AppState;
use crate::services::memory_import;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/instances/{instance_slug}/memory/import",
            post(start_import),
        )
        .layer(DefaultBodyLimit::max(500 * 1024 * 1024)) // 500 MB
}

#[derive(Serialize)]
struct ImportStarted {
    ok: bool,
    message: String,
}

/// POST /api/instances/:slug/memory/import
/// Accepts multipart file upload — can be a single JSON, or multiple files.
/// Saves files to a temp dir and spawns the import pipeline.
async fn start_import(
    State(state): State<AppState>,
    Path(instance_slug): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<ImportStarted>, (axum::http::StatusCode, String)> {
    // Get API key from config
    let api_key = {
        let config = state.config.read().await;
        config.llm.api_key().map(|s| s.to_string())
    };
    let api_key = api_key.ok_or_else(|| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "LLM not configured".to_string(),
        )
    })?;

    // Create temp dir for uploaded files
    let import_dir = state
        .workspace_dir
        .join("instances")
        .join(&instance_slug)
        .join(".import_temp");
    let _ = std::fs::remove_dir_all(&import_dir); // Clean previous
    std::fs::create_dir_all(&import_dir).map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to create import dir: {e}"),
        )
    })?;

    // Save uploaded files
    let mut file_count = 0;
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.file_name().unwrap_or("data.bin").to_string();

        let data = field.bytes().await.map_err(|e| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                format!("failed to read field: {e}"),
            )
        })?;

        if data.is_empty() {
            continue;
        }

        let file_path = import_dir.join(&name);
        // If name contains path separators, create parent dirs
        if let Some(parent) = file_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&file_path, &data).map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to write file: {e}"),
            )
        })?;
        file_count += 1;
    }

    if file_count == 0 {
        let _ = std::fs::remove_dir_all(&import_dir);
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "no files uploaded".to_string(),
        ));
    }

    // Spawn the background import pipeline
    memory_import::spawn_import(
        state.http_client.clone(),
        api_key,
        state.workspace_dir.clone(),
        instance_slug.clone(),
        import_dir,
        state.events.clone(),
        state.vector_store.clone(),
    );

    Ok(Json(ImportStarted {
        ok: true,
        message: format!(
            "import started with {file_count} file(s) — progress updates via WebSocket"
        ),
    }))
}
