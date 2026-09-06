use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::get;
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use super::auth::AuthUser;
use super::errors::ApiError;
use super::visibility::{self, Visibility};
use crate::state::{AlarmEvent, AppState, NodeEvent};
use crate::templates::{AlarmStub, NodeStatusFragment};

/// All routes serving the live channel.
pub fn routes() -> Router<AppState> {
    Router::new().route("/api/ws", get(upgrade))
}

async fn upgrade(
    auth: AuthUser,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let access = visibility::resolve(&state, auth.session.user_id).await?;

    Ok(ws.on_upgrade(move |socket| run(socket, state, access.visibility)))
}

async fn run(socket: WebSocket, state: AppState, visibility: Visibility) {
    let (mut sender, mut receiver) = socket.split();
    let mut node_events = state.node_events.subscribe();
    let mut alarm_events = state.alarm_events.subscribe();

    loop {
        tokio::select! {
            incoming = receiver.next() => match incoming {
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            },

            event = node_events.recv() => match event {
                Ok(event) => {
                    if visibility.allows(event.node_id())
                        && send(&mut sender, node_fragment(&event)).await.is_err()
                    {
                        break;
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "node event receiver lagged, dropping the gap");
                }
                Err(RecvError::Closed) => break,
            },

            event = alarm_events.recv() => match event {
                Ok(event) => {
                    if visibility.allows(event.node_id())
                        && send(&mut sender, alarm_fragment(&event)).await.is_err()
                    {
                        break;
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "alarm event receiver lagged, dropping the gap");
                }
                Err(RecvError::Closed) => break,
            },
        }
    }
}

async fn send(
    sender: &mut SplitSink<WebSocket, Message>,
    fragment: Result<String, askama::Error>,
) -> Result<(), axum::Error> {
    match fragment {
        Ok(html) => sender.send(Message::Text(html.into())).await,
        Err(err) => {
            tracing::error!(error = %err, "failed to render a live fragment");
            Ok(())
        }
    }
}

fn node_fragment(event: &NodeEvent) -> Result<String, askama::Error> {
    let (state, at) = match *event {
        NodeEvent::Reported { at, .. } => ("reporting", crate::templates::stamp(at)),
        NodeEvent::Silent { since, .. } => ("silent", crate::templates::stamp(since)),
        NodeEvent::SuspensionChanged { suspended, .. } => (
            if suspended { "suspended" } else { "reporting" },
            crate::templates::Stamp::default(),
        ),
    };

    NodeStatusFragment {
        node_id: event.node_id(),
        state,
        at,
    }
    .render()
}

fn alarm_fragment(event: &AlarmEvent) -> Result<String, askama::Error> {
    AlarmStub {
        alarm_id: event.alarm_id(),
        replace: matches!(*event, AlarmEvent::StateChanged { .. }),
    }
    .render()
}
