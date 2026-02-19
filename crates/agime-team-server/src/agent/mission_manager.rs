//! Mission manager for tracking active mission executions
//!
//! Similar to ChatManager but for Mission Track (Phase 2).
//! Tracks cancellation tokens and broadcast channels for active missions.

use super::task_manager::StreamEvent;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::{broadcast, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Active mission execution info
pub struct ActiveMission {
    pub mission_id: String,
    pub cancel_token: CancellationToken,
    pub stream_tx: broadcast::Sender<StreamEvent>,
    pub started_at: std::time::Instant,
    pub last_activity: Arc<Mutex<std::time::Instant>>,
}

/// Mission manager for tracking active mission executions
pub struct MissionManager {
    missions: RwLock<HashMap<String, ActiveMission>>,
}

impl MissionManager {
    pub fn new() -> Self {
        Self {
            missions: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new active mission, returns (CancellationToken, broadcast::Sender).
    /// Returns None if the mission is already active.
    pub async fn register(
        &self,
        mission_id: &str,
    ) -> Option<(CancellationToken, broadcast::Sender<StreamEvent>)> {
        let mut missions = self.missions.write().await;
        if missions.contains_key(mission_id) {
            warn!("Mission already active, rejecting register: {}", mission_id);
            return None;
        }

        let token = CancellationToken::new();
        let (tx, _) = broadcast::channel(512);
        let now = std::time::Instant::now();
        let mission = ActiveMission {
            mission_id: mission_id.to_string(),
            cancel_token: token.clone(),
            stream_tx: tx.clone(),
            started_at: now,
            last_activity: Arc::new(Mutex::new(now)),
        };

        missions.insert(mission_id.to_string(), mission);
        info!("Mission registered: {}", mission_id);
        Some((token, tx))
    }

    /// Subscribe to mission stream events
    pub async fn subscribe(&self, mission_id: &str) -> Option<broadcast::Receiver<StreamEvent>> {
        let missions = self.missions.read().await;
        missions.get(mission_id).map(|m| m.stream_tx.subscribe())
    }

    /// Broadcast an event to mission subscribers
    pub async fn broadcast(&self, mission_id: &str, event: StreamEvent) {
        let missions = self.missions.read().await;
        if let Some(m) = missions.get(mission_id) {
            if let Ok(mut t) = m.last_activity.lock() {
                *t = std::time::Instant::now();
            }
            let _ = m.stream_tx.send(event);
        }
    }

    /// Mark mission as completed and remove from tracking
    pub async fn complete(&self, mission_id: &str) {
        let mut missions = self.missions.write().await;
        if let Some(m) = missions.remove(mission_id) {
            let duration = m.started_at.elapsed();
            info!(
                "Mission completed: {} (duration: {:?})",
                mission_id, duration
            );
        }
    }

    /// Cancel an active mission and remove from tracking
    pub async fn cancel(&self, mission_id: &str) -> bool {
        let mut missions = self.missions.write().await;
        if let Some(m) = missions.remove(mission_id) {
            m.cancel_token.cancel();
            warn!("Mission cancelled and removed: {}", mission_id);
            true
        } else {
            false
        }
    }

    /// Check if a mission is active
    pub async fn is_active(&self, mission_id: &str) -> bool {
        let missions = self.missions.read().await;
        missions.contains_key(mission_id)
    }

    /// Clean up stale missions with no activity longer than max_age
    pub async fn cleanup_stale(&self, max_age: std::time::Duration) -> usize {
        let mut missions = self.missions.write().await;
        let before = missions.len();
        let now = std::time::Instant::now();
        missions.retain(|mid, m| {
            let last = m.last_activity.lock().ok().map(|t| *t).unwrap_or(m.started_at);
            let inactive_for = now.saturating_duration_since(last);
            let stale = inactive_for > max_age;
            if stale {
                m.cancel_token.cancel();
                warn!(
                    "Cleaning up stale mission: {} (inactive_for: {:?})",
                    mid, inactive_for
                );
            }
            !stale
        });
        let removed = before - missions.len();
        if removed > 0 {
            info!("Cleaned up {} stale missions", removed);
        }
        removed
    }

    /// Get count of active missions
    pub async fn active_count(&self) -> usize {
        let missions = self.missions.read().await;
        missions.len()
    }
}

impl super::runtime::EventBroadcaster for MissionManager {
    async fn broadcast(&self, context_id: &str, event: StreamEvent) {
        self.broadcast(context_id, event).await;
    }
}

impl Default for MissionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a shared mission manager
#[allow(dead_code)]
pub fn create_mission_manager() -> Arc<MissionManager> {
    Arc::new(MissionManager::new())
}
