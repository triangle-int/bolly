//! Request-admission boundary for the one canonical companion (#103).
//!
//! Applied as a route layer to every router that carries an `instance_slug`
//! path parameter. Foreign slugs fail closed with `404 unknown_companion`
//! before any handler runs, so no request can create, read, or delete a
//! second companion. Canonical reads never create storage; canonical writes
//! create-or-open the companion through the identity marker, and an
//! unsupported marker refuses both with `503 companion_format_unsupported`.

use std::{collections::HashMap, path::Path};

use axum::{
    Json,
    extract::{Path as PathParams, Request, State, rejection::PathRejection},
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{
    app::state::AppState,
    domain::companion::{CANONICAL_SLUG, IdentityError, is_canonical},
    services::companion,
};

/// Path parameter name shared by every companion-scoped route.
pub const SLUG_PARAM: &str = "instance_slug";

/// Why a request was not admitted to the companion.
#[derive(Debug)]
pub enum CompanionRejection {
    /// The slug names something other than the one companion this server owns.
    UnknownCompanion,
    /// The canonical companion exists but its storage cannot be trusted.
    Identity(IdentityError),
}

impl From<IdentityError> for CompanionRejection {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}

impl IntoResponse for CompanionRejection {
    fn into_response(self) -> Response {
        match self {
            Self::UnknownCompanion => (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "unknown_companion",
                    "message": format!("this server owns one companion: {CANONICAL_SLUG}"),
                })),
            )
                .into_response(),
            Self::Identity(IdentityError::UnsupportedFormat(message)) => (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": "companion_format_unsupported",
                    "message": message,
                })),
            )
                .into_response(),
            Self::Identity(IdentityError::Io(message)) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "companion_storage_error",
                    "message": message,
                })),
            )
                .into_response(),
        }
    }
}

/// Admit a request addressed to `slug`.
///
/// Reads (`GET`, `HEAD`, `OPTIONS`) and `DELETE` never create storage. Any
/// other method creates-or-opens the canonical companion first.
pub fn admit(workspace_dir: &Path, slug: &str, method: &Method) -> Result<(), CompanionRejection> {
    if !is_canonical(slug) {
        return Err(CompanionRejection::UnknownCompanion);
    }
    let read_only = matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::DELETE
    );
    if read_only {
        companion::read_identity(workspace_dir)?;
    } else {
        companion::ensure_identity(workspace_dir)?;
    }
    Ok(())
}

pub async fn companion_boundary(
    State(state): State<AppState>,
    params: Result<PathParams<HashMap<String, String>>, PathRejection>,
    request: Request,
    next: Next,
) -> Response {
    if let Ok(PathParams(params)) = params
        && let Some(slug) = params.get(SLUG_PARAM)
        && let Err(rejection) = admit(&state.workspace_dir, slug, request.method())
    {
        return rejection.into_response();
    }
    next.run(request).await
}
