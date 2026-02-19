//! Chat session manager for tracking active chat sessions
//!
//! Similar to TaskManager but for direct chat sessions that bypass the Task system.
//! Tracks cancellation tokens and broadcast channels for active chats.

use super::task_manager::StreamEvent;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::{broadcast, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const EVENT_HISTORY_LIMIT: usize = 400;

/// Stream event with monotonically increasing sequence id.
#[derive(Clone, Debug)]
pub struct ChatStreamEvent {
    pub id: u64,
    pub event: StreamEvent,
}

struct EventBuffer {
    next_id: u64,
    events: VecDeque<ChatStreamEvent>,
    last_activity: std::time::Instant,
}

/// Active chat session info
pub struct ActiveChat {
    pub cancel_token: CancellationToken,
    pub stream_tx: broadcast::Sender<ChatStreamEvent>,
    buffer: Arc<Mutex<EventBuffer>>,
    pub started_at: std::time::Instant,
}

/// Chat manager for tracking active chat sessions
pub struct ChatManager {
    chats: RwLock<HashMap<String, ActiveChat>>,
}

impl ChatManager {
    pub fn new() -> Self {
        Self {
            chats: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new active chat, returns (CancellationToken, broadcast::Sender).
    /// Returns None if the session is already active (prevents overwrite).
    pub async fn register(
        &self,
        session_id: &str,
    ) -> Option<(CancellationToken, broadcast::Sender<ChatStreamEvent>)> {
        let mut chats = self.chats.write().await;

        // Reject if session is already active
        if chats.contains_key(session_id) {
            warn!(
                "Chat session already active, rejecting register: {}",
                session_id
            );
            return None;
        }

        let token = CancellationToken::new();
        let (tx, _) = broadcast::channel(512);
        let now = std::time::Instant::now();
        let chat = ActiveChat {
            cancel_token: token.clone(),
            stream_tx: tx.clone(),
            buffer: Arc::new(Mutex::new(EventBuffer {
                next_id: 1,
                events: VecDeque::with_capacity(EVENT_HISTORY_LIMIT),
                last_activity: now,
            })),
            started_at: now,
        };

        chats.insert(session_id.to_string(), chat);
        info!("Chat session registered: {}", session_id);

        Some((token, tx))
    }

    /// Subscribe to chat stream events
    pub async fn subscribe(
        &self,
        session_id: &str,
    ) -> Option<broadcast::Receiver<ChatStreamEvent>> {
        let chats = self.chats.read().await;
        chats.get(session_id).map(|c| c.stream_tx.subscribe())
    }

    /// Subscribe to chat stream and return recent buffered events newer than `after_id`.
    pub async fn subscribe_with_history(
        &self,
        session_id: &str,
        after_id: Option<u64>,
    ) -> Option<(broadcast::Receiver<ChatStreamEvent>, Vec<ChatStreamEvent>)> {
        let chats = self.chats.read().await;
        let chat = chats.get(session_id)?;
        let rx = chat.stream_tx.subscribe();
        let history = chat.buffer.lock().ok().map_or_else(Vec::new, |b| {
            if let Some(after) = after_id {
                b.events
                    .iter()
                    .filter(|it| it.id > after)
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                b.events.iter().cloned().collect::<Vec<_>>()
            }
        });
        Some((rx, history))
    }

    /// Broadcast an event to chat subscribers
    pub async fn broadcast(&self, session_id: &str, event: StreamEvent) {
        let chats = self.chats.read().await;
        if let Some(chat) = chats.get(session_id) {
            if let Ok(mut buffer) = chat.buffer.lock() {
                buffer.last_activity = std::time::Instant::now();
                let event_for_item = event.clone();
                let item = ChatStreamEvent {
                    id: buffer.next_id,
                    event: event_for_item,
                };
                buffer.next_id = buffer.next_id.saturating_add(1);
                buffer.events.push_back(item.clone());
                while buffer.events.len() > EVENT_HISTORY_LIMIT {
                    let _ = buffer.events.pop_front();
                }
                let _ = chat.stream_tx.send(item);
            } else {
                // Fallback when lock is poisoned: still push event without id tracking.
                let _ = chat.stream_tx.send(ChatStreamEvent { id: 0, event });
            }
        }
    }

    /// Mark chat as completed and remove from tracking
    pub async fn complete(&self, session_id: &str) {
        let mut chats = self.chats.write().await;
        if let Some(chat) = chats.remove(session_id) {
            let duration = chat.started_at.elapsed();
            info!(
                "Chat session completed: {} (duration: {:?})",
                session_id, duration
            );
        }
    }

    /// Remove a session from tracking without cancelling its token.
    /// Used for rollback when the DB step fails after in-memory registration.
    pub async fn unregister(&self, session_id: &str) {
        let mut chats = self.chats.write().await;
        if chats.remove(session_id).is_some() {
            warn!("Chat session unregistered (rollback): {}", session_id);
        }
    }

    /// Cancel an active chat session and remove it from tracking
    pub async fn cancel(&self, session_id: &str) -> bool {
        let mut chats = self.chats.write().await;
        if let Some(chat) = chats.remove(session_id) {
            chat.cancel_token.cancel();
            warn!("Chat session cancelled and removed: {}", session_id);
            true
        } else {
            false
        }
    }

    /// Check if a chat session is active
    pub async fn is_active(&self, session_id: &str) -> bool {
        let chats = self.chats.read().await;
        chats.contains_key(session_id)
    }

    /// M1: Clean up stale entries that have been running longer than the timeout.
    /// This prevents leaked entries from blocking new chats on the same session.
    pub async fn cleanup_stale(&self, max_age: std::time::Duration) -> usize {
        let mut chats = self.chats.write().await;
        let before = chats.len();
        let now = std::time::Instant::now();
        chats.retain(|sid, chat| {
            let last_activity = chat
                .buffer
                .lock()
                .ok()
                .map(|b| b.last_activity)
                .unwrap_or(chat.started_at);
            let inactive_for = now.saturating_duration_since(last_activity);
            let stale = inactive_for > max_age;
            if stale {
                chat.cancel_token.cancel();
                warn!(
                    "Cleaning up stale chat session: {} (inactive_for: {:?})",
                    sid, inactive_for
                );
            }
            !stale
        });
        let removed = before - chats.len();
        if removed > 0 {
            info!("Cleaned up {} stale chat sessions", removed);
        }
        removed
    }

    /// Get count of active chat sessions
    pub async fn active_count(&self) -> usize {
        let chats = self.chats.read().await;
        chats.len()
    }
}

impl super::runtime::EventBroadcaster for ChatManager {
    async fn broadcast(&self, context_id: &str, event: StreamEvent) {
        self.broadcast(context_id, event).await;
    }
}

impl Default for ChatManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a shared chat manager
#[allow(dead_code)]
pub fn create_chat_manager() -> Arc<ChatManager> {
    Arc::new(ChatManager::new())
}
