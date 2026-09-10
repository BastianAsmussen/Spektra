use std::time::Duration;

use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::response::Response;
use axum::routing::get;
use futures_util::sink::SinkExt;
use futures_util::stream::{SplitSink, StreamExt};
use tokio::sync::broadcast::error::RecvError;

use super::auth::{self, AuthUser};
use super::errors::ApiError;
use super::visibility::{self, Visibility};
use crate::live;
use crate::state::{AlarmEvent, AppState, NodeEvent};
use crate::templates::{AlarmStub, LiveReading, LiveReadings, NodeStatusFragment};

const REVALIDATE: Duration = Duration::from_mins(1);

/// Close reason the browser matches on to send the reader to the login form.
pub const SESSION_ENDED: &str = "session-expired";

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
    let token = auth.session.token;

    Ok(ws.on_upgrade(move |socket| run(socket, state, access.visibility, token)))
}

async fn run(socket: WebSocket, state: AppState, visibility: Visibility, token: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut node_events = state.node_events.subscribe();
    let mut alarm_events = state.alarm_events.subscribe();
    let mut live_samples = state.live_samples.subscribe();

    let mut revalidate = tokio::time::interval(REVALIDATE);
    revalidate.tick().await;

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

            sample = live_samples.recv() => match sample {
                Ok(sample) => {
                    if visibility.allows(sample.node_id)
                        && send(&mut sender, live_fragment(&sample)).await.is_err()
                    {
                        break;
                    }
                }
                Err(RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "live sample receiver lagged, dropping the gap");
                }
                Err(RecvError::Closed) => break,
            },

            _ = revalidate.tick() => {
                if !auth::session_is_live(&state, &token).await {
                    tracing::info!("the session behind an open socket ended, closing it");
                    drop(sender.send(Message::Close(Some(CloseFrame {
                        code: close_code::NORMAL,
                        reason: SESSION_ENDED.into(),
                    }))).await);

                    break;
                }
            }
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

fn live_fragment(sample: &live::Sample) -> Result<String, askama::Error> {
    LiveReadings {
        node_id: sample.node_id,
        slug: crate::api::nodes::channel_slug(sample.frequency_hz),
        label: sample.label.clone(),
        at: crate::templates::stamp(sample.measured_at.naive_utc()),
        readings: sample
            .readings
            .iter()
            .map(|&(metric, value)| {
                let (name, value) = crate::api::series::reading(metric, value);

                LiveReading { name, value }
            })
            .collect(),
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
