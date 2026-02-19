//! SSE streaming for task execution results
//!
//! This module provides Server-Sent Events (SSE) streaming
//! for real-time task execution result updates using broadcast channels.

use axum::response::sse::{Event, Sse};
use futures::stream::Stream;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

use super::task_manager::{StreamEvent, TaskManager};

/// Stream task results via SSE using broadcast channel (real-time)
pub fn stream_task_results(
    task_id: String,
    task_manager: Arc<TaskManager>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = async_stream::stream! {
        // Try to subscribe to the task's broadcast channel
        let mut receiver = match task_manager.subscribe(&task_id).await {
            Some(rx) => rx,
            None => {
                // Task not found or already completed
                yield Ok(Event::default()
                    .event("done")
                    .data(serde_json::json!({
                        "type": "Done",
                        "status": "not_found",
                        "error": "Task not found or already completed"
                    }).to_string()));
                return;
            }
        };

        // Send initial status
        yield Ok(Event::default()
            .event("status")
            .data(serde_json::json!({
                "type": "Status",
                "status": "running"
            }).to_string()));

        // Maximum SSE connection lifetime to prevent zombie connections
        let sse_lifetime_secs = std::env::var("TEAM_SSE_MAX_LIFETIME_SECS")
            .ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(2 * 60 * 60);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(sse_lifetime_secs);

        loop {
            match tokio::time::timeout_at(deadline, receiver.recv()).await {
                Ok(Ok(event)) => {
                    let is_done = event.is_done();
                    let data = serde_json::to_string(&event).unwrap_or_default();
                    yield Ok(Event::default().event(event.event_type()).data(data));

                    if is_done {
                        break;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                    tracing::warn!("SSE task stream lagged by {} messages", n);
                    continue;
                }
                Err(_) => {
                    tracing::info!("SSE stream for task {} deadline reached, closing for client reconnect", task_id);
                    break;
                }
            }
        }
    };

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("ping"),
    )
}
