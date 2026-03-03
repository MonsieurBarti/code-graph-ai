use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;

use super::server::AppState;

/// WebSocket upgrade handler.
///
/// Upgrades the HTTP connection to WebSocket and spawns `handle_socket`
/// to forward broadcast messages to the client.
pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle an individual WebSocket connection.
///
/// Subscribes to the broadcast channel and forwards every message to the
/// connected client as a `Text` frame. Incoming client messages are ignored.
/// Exits cleanly on send error or client disconnect.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.ws_tx.subscribe();

    loop {
        tokio::select! {
            // Broadcast message from the watcher task.
            msg_result = rx.recv() => {
                match msg_result {
                    Ok(msg) => {
                        if socket.send(Message::Text(msg.into())).await.is_err() {
                            // Client disconnected or send failed — exit.
                            break;
                        }
                    }
                    Err(_) => {
                        // Channel lagged or closed.
                        break;
                    }
                }
            }
            // Receive (and ignore) messages from the client.
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(_)) => {
                        // Ignore client messages in this phase.
                    }
                    // Client disconnected or error.
                    _ => break,
                }
            }
        }
    }
}
