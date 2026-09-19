use std::time::Duration;

use axum::{
    Extension, Router,
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
    routing::get,
};

use crate::app::{auth::AuthContext, state::AppState};

/// Close code sent when the browser session behind a socket is revoked.
pub const CLOSE_SESSION_REVOKED: u16 = 4401;
const SESSION_CHECK_INTERVAL: Duration = Duration::from_secs(30);

pub fn router() -> Router<AppState> {
    Router::new().route("/api/ws", get(upgrade))
}

async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    context: Option<Extension<AuthContext>>,
) -> Response {
    let session_id = match context {
        Some(Extension(AuthContext::BrowserSession { id })) => Some(id),
        _ => None,
    };
    ws.on_upgrade(move |socket| handle_socket(socket, state, session_id))
}

async fn handle_socket(mut socket: WebSocket, state: AppState, session_id: Option<String>) {
    let mut receiver = state.events.subscribe();
    let mut session_check = tokio::time::interval(SESSION_CHECK_INTERVAL);
    session_check.tick().await; // the first tick fires immediately

    loop {
        tokio::select! {
            _ = session_check.tick() => {
                if let Some(id) = &session_id
                    && !state.browser_sessions.is_active(id)
                {
                    let _ = socket
                        .send(Message::Close(Some(CloseFrame {
                            code: CLOSE_SESSION_REVOKED,
                            reason: "session revoked".into(),
                        })))
                        .await;
                    break;
                }
            }
            event = receiver.recv() => {
                match event {
                    Ok(event) => {
                        let payload = match serde_json::to_string(&event) {
                            Ok(payload) => payload,
                            Err(_) => continue,
                        };

                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("[ws] client lagged, dropped {n} events — sending resync hint");
                        let hint = r#"{"type":"resync","reason":"lagged"}"#;
                        let _ = socket.send(Message::Text(hint.into())).await;
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}
