use axum::{
    Json, Router,
    extract::Query,
    http::header,
    response::{Html, IntoResponse},
    routing::get,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
struct AuthBody {
    token: Option<String>,
}

fn auth_html(token: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Authenticating...</title></head>
<body>
<script>
localStorage.setItem("nolune_auth_token", {token});
window.location.replace("/");
</script>
<noscript>JavaScript is required.</noscript>
</body></html>"#,
        token = serde_json::to_string(token).unwrap_or_default()
    )
}

async fn auth_get(Query(q): Query<AuthQuery>) -> impl IntoResponse {
    let token = q.token.unwrap_or_default();
    (
        [(header::REFERRER_POLICY, "no-referrer")],
        Html(auth_html(&token)),
    )
}

async fn auth_post(Json(body): Json<AuthBody>) -> Html<String> {
    let token = body.token.unwrap_or_default();
    Html(auth_html(&token))
}

pub fn router() -> Router<crate::app::state::AppState> {
    Router::new().route("/auth", get(auth_get).post(auth_post))
}
