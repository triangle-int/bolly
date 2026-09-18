use axum::{
    body::Body,
    http::{Request, Response, StatusCode, header},
    response::IntoResponse,
};
use rust_embed::Embed;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::Service;

#[derive(Embed)]
#[folder = "../client/build"]
struct StaticAssets;

/// Tower service that serves embedded static files with SPA fallback.
#[derive(Clone)]
pub struct EmbeddedStaticService;

impl Service<Request<Body>> for EmbeddedStaticService {
    type Response = Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let path = req.uri().path().trim_start_matches('/').to_string();
        let resp = serve_embedded(&path);
        Box::pin(async move { Ok(resp) })
    }
}

fn serve_embedded(path: &str) -> Response<Body> {
    // Try exact file match
    if let Some(file) = StaticAssets::get(path) {
        let mime = file.metadata.mimetype().to_string();
        let cc = cache_control(path);
        return Response::builder()
            .header(header::CONTENT_TYPE, mime)
            .header(header::CACHE_CONTROL, cc)
            .body(Body::from(file.data.to_vec()))
            .unwrap();
    }

    // SPA fallback — serve index.html
    if let Some(index) = StaticAssets::get("index.html") {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from(index.data.to_vec()))
            .unwrap();
    }

    StatusCode::NOT_FOUND.into_response()
}

fn cache_control(path: &str) -> &'static str {
    if path.starts_with("_app/") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=60"
    }
}
