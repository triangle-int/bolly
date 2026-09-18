use super::*;
use crate::config::Config;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    routing::post,
};
use std::sync::{Arc, Mutex};

type Requests = Arc<Mutex<Vec<(String, Option<String>, serde_json::Value)>>>;
type Replies = Arc<Mutex<std::collections::VecDeque<(u16, serde_json::Value)>>>;

pub(crate) struct MockServer {
    pub config: Config,
    pub requests: Arc<Mutex<Vec<(String, Option<String>, serde_json::Value)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl MockServer {
    pub async fn new(responses: Vec<(u16, serde_json::Value)>) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(std::collections::VecDeque::from(responses)));
        let router = Router::new()
            .route(
                "/v1/embeddings",
                post(
                    |State((requests, replies)): State<(Requests, Replies)>,
                     uri: Uri,
                     headers: HeaderMap,
                     Json(body): Json<serde_json::Value>| async move {
                        requests.lock().unwrap().push((
                            uri.to_string(),
                            headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            body,
                        ));
                        let (status, body) = replies
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or((503, serde_json::json!({"error":"unexpected request"})));
                        (StatusCode::from_u16(status).unwrap(), Json(body))
                    },
                ),
            )
            .with_state((requests.clone(), replies));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = Config::default();
        config.embedding.provider = "openai_compatible".into();
        config.embedding.base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        config.embedding.dimensions = 3;
        config.llm.tokens.open_ai = "mock-secret".into();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            config,
            requests,
            task,
        }
    }
}

pub(crate) fn response(vector: Vec<f32>) -> serde_json::Value {
    serde_json::json!({"data":[{"index":0,"embedding":vector}]})
}

#[tokio::test]
async fn compatible_document_and_query_requests_are_unauthenticated() {
    let mock = MockServer::new(vec![
        (200, response(vec![1., 2., 3.])),
        (200, response(vec![3., 2., 1.])),
    ])
    .await;
    let service = EmbeddingService::from_config(&mock.config);
    assert_eq!(service.status()["status"], "unverified");
    assert_eq!(
        service.document("a memory").await.unwrap(),
        vec![1., 2., 3.]
    );
    assert_eq!(service.query("a question").await.unwrap(), vec![3., 2., 1.]);
    assert_eq!(service.status()["status"], "available");
    let requests = mock.requests.lock().unwrap();
    for (i, (path, auth, body)) in requests.iter().enumerate() {
        assert_eq!(path, "/v1/embeddings");
        assert_eq!(auth, &None);
        assert_eq!(
            body,
            &serde_json::json!({"model":"text-embedding-3-small", "input":[if i==0 {"a memory"} else {"a question"}], "dimensions":3})
        );
    }
}

#[test]
fn official_openai_request_uses_the_stored_bearer_key() {
    let mut config = Config::default();
    config.llm.tokens.open_ai = "mock-secret".into();
    let provider =
        OpenAiEmbeddingProvider::from_config(&config.embedding, &config.llm.tokens.open_ai)
            .unwrap();
    let request = provider
        .build_request(&["memory".to_owned()])
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        request.url().as_str(),
        "https://api.openai.com/v1/embeddings"
    );
    assert_eq!(request.headers()["authorization"], "Bearer mock-secret");
}

#[tokio::test]
async fn rejected_compatible_endpoints_are_never_contacted() {
    let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
    let local = mock.config.embedding.base_url.clone();
    for endpoint in [
        "http://10.0.0.1/v1".to_owned(),
        "http://192.168.1.2/v1".to_owned(),
        "http://169.254.169.254/latest".to_owned(),
        "http://[fe80::1]/v1".to_owned(),
        "https://example.com/v1".to_owned(),
        format!("{local}?token=secret"),
        format!("{local}#fragment"),
        local.replacen("http://", "http://user:secret@", 1),
    ] {
        let mut config = mock.config.clone();
        config.embedding.base_url = endpoint;
        let service = EmbeddingService::from_config(&config);
        assert!(service.query("must not leave the process").await.is_err());
    }
    assert!(mock.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn openai_rejects_http_api_shape_count_dimension_and_nonfinite_errors_safely() {
    let cases = vec![
        (
            401,
            serde_json::json!({"error":{"message":"mock-secret auth"}}),
        ),
        (503, serde_json::json!({"error":"mock-secret outage"})),
        (200, serde_json::json!({"error":{"message":"mock-secret"}})),
        (200, serde_json::json!({"data":[]})),
        (
            200,
            serde_json::json!({"data":[{"index":0,"embedding":[1,2,3]},{"index":1,"embedding":[1,2,3]}]}),
        ),
        (200, response(vec![1., 2.])),
        (
            200,
            serde_json::json!({"data":[{"index":0,"embedding":[1e100,2,3]}]}),
        ),
        (
            200,
            serde_json::json!({"data":[{"index":1,"embedding":[1,2,3]}]}),
        ),
        (
            200,
            serde_json::json!({"data":[{"index":0,"embedding":[0,0,0]}]}),
        ),
    ];
    for case in cases {
        let mock = MockServer::new(vec![case, (200, response(vec![1., 0., 0.]))]).await;
        let service = EmbeddingService::from_config(&mock.config);
        let err = service.query("query").await.unwrap_err();
        assert!(!err.contains("mock-secret"));
        assert_eq!(service.status()["status"], "unavailable");
        assert!(!service.status().to_string().contains("mock-secret"));
        // Failures are retryable; success restores the status.
        service.query("retry").await.unwrap();
        assert_eq!(service.status()["status"], "available");
    }
}

#[tokio::test]
async fn embedding_requests_enforce_input_count_and_byte_limits_before_network_io() {
    let mock = MockServer::new(vec![(200, response(vec![1., 0., 0.]))]).await;
    let provider = OpenAiEmbeddingProvider::from_config(&mock.config.embedding, "ignored").unwrap();

    let too_many = vec!["x".to_owned(); 65];
    assert!(
        provider
            .documents(&too_many)
            .await
            .unwrap_err()
            .contains("count")
    );
    assert!(
        provider
            .query(&"é".repeat(4097))
            .await
            .unwrap_err()
            .contains("input")
    );
    let too_large_total = vec!["x".repeat(8192); 9];
    assert!(
        provider
            .documents(&too_large_total)
            .await
            .unwrap_err()
            .contains("total")
    );
    assert!(mock.requests.lock().unwrap().is_empty());
}

async fn raw_response_server(response: Vec<u8>) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 16 * 1024];
        let _ = socket.read(&mut request).await;
        socket.write_all(&response).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    (format!("http://{address}/v1"), task)
}

#[tokio::test]
async fn embedding_response_rejects_oversized_content_length_before_reading_body() {
    let raw =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000000\r\n\r\n"
            .to_vec();
    let (base_url, task) = raw_response_server(raw).await;
    let mut config = Config::default();
    config.embedding.provider = "openai_compatible".into();
    config.embedding.base_url = base_url;
    config.embedding.dimensions = 3;
    let error = EmbeddingService::from_config(&config)
        .query("query")
        .await
        .unwrap_err();
    assert_eq!(error, "embedding response is too large");
    task.await.unwrap();
}

#[tokio::test]
async fn embedding_response_rejects_oversized_chunked_body_while_streaming() {
    let chunk = vec![b'x'; 8192];
    let mut raw =
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n"
            .to_vec();
    raw.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
    raw.extend_from_slice(&chunk);
    raw.extend_from_slice(b"\r\n0\r\n\r\n");
    let (base_url, task) = raw_response_server(raw).await;
    let mut config = Config::default();
    config.embedding.provider = "openai_compatible".into();
    config.embedding.base_url = base_url;
    config.embedding.dimensions = 3;
    let error = EmbeddingService::from_config(&config)
        .query("query")
        .await
        .unwrap_err();
    assert_eq!(error, "embedding response is too large");
    task.await.unwrap();
}

#[tokio::test]
async fn missing_disabled_or_unreachable_backend_is_unavailable() {
    let mut config = Config::default();
    let service = EmbeddingService::from_config(&config);
    assert!(service.document("memory").await.is_err());
    config.embedding.enabled = false;
    config.llm.tokens.open_ai = "mock-secret".into();
    assert!(
        EmbeddingService::from_config(&config)
            .query("q")
            .await
            .is_err()
    );
    let mock = MockServer::new(vec![]).await;
    config = mock.config.clone();
    drop(mock);
    let service = EmbeddingService::from_config(&config);
    assert!(service.query("q").await.is_err());
    assert_eq!(service.status()["status"], "unavailable");
}
