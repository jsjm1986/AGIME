//! Mission manager for tracking active mission executions
//!
//! Similar to ChatManager but for Mission Track (Phase 2).
//! Tracks cancellation tokens and broadcast channels for active missions.

use super::service_mongo::AgentService;
use super::task_manager::StreamEvent;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

const EVENT_HISTORY_LIMIT: usize = 400;
const EVENT_PERSIST_BATCH_SIZE: usize = 128;
const EVENT_PERSIST_FLUSH_MS: u64 = 25;
const EVENT_PERSIST_QUEUE_CAP: usize = 8192;

/// Stream event with monotonically increasing sequence id.
#[derive(Clone, Debug)]
pub struct MissionStreamEvent {
    pub id: u64,
    pub event: StreamEvent,
}

struct EventBuffer {
    next_id: u64,
    events: VecDeque<MissionStreamEvent>,
    last_activity: std::time::Instant,
}

#[derive(Clone, Debug)]
struct PersistMissionEvent {
    mission_id: String,
    run_id: String,
    event_id: u64,
    event: StreamEvent,
}

pub struct MissionRegistration {
    pub cancel_token: CancellationToken,
    #[allow(dead_code)]
    pub stream_tx: broadcast::Sender<MissionStreamEvent>,
    pub run_id: String,
}

/// Active mission execution info
pub struct ActiveMission {
    pub mission_id: String,
    pub run_id: String,
    pub cancel_token: CancellationToken,
    pub stream_tx: broadcast::Sender<MissionStreamEvent>,
    buffer: Arc<Mutex<EventBuffer>>,
    pub started_at: std::time::Instant,
    pub last_activity: Arc<Mutex<std::time::Instant>>,
}

/// Mission manager for tracking active mission executions
pub struct MissionManager {
    missions: RwLock<HashMap<String, ActiveMission>>,
    persist_tx: Option<mpsc::Sender<PersistMissionEvent>>,
}

impl MissionManager {
    pub fn new() -> Self {
        Self {
            missions: RwLock::new(HashMap::new()),
            persist_tx: None,
        }
    }

    pub fn new_with_event_persistence(agent_service: Arc<AgentService>) -> Self {
        let (tx, mut rx) = mpsc::channel::<PersistMissionEvent>(EVENT_PERSIST_QUEUE_CAP);
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch = Vec::with_capacity(EVENT_PERSIST_BATCH_SIZE);
                batch.push(first);
                let deadline =
                    tokio::time::Instant::now() + Duration::from_millis(EVENT_PERSIST_FLUSH_MS);
                while batch.len() < EVENT_PERSIST_BATCH_SIZE {
                    match tokio::time::timeout_at(deadline, rx.recv()).await {
                        Ok(Some(item)) => batch.push(item),
                        _ => break,
                    }
                }
                let records: Vec<(String, String, u64, StreamEvent)> = batch
                    .into_iter()
                    .map(|item| (item.mission_id, item.run_id, item.event_id, item.event))
                    .collect();
                if let Err(e) = agent_service.save_mission_stream_events(&records).await {
                    warn!(
                        "Failed to persist mission stream events ({}): {}",
                        records.len(),
                        e
                    );
                }
            }
            info!("Mission event persistence worker stopped");
        });

        Self {
            missions: RwLock::new(HashMap::new()),
            persist_tx: Some(tx),
        }
    }

    /// Register a new active mission, returns cancellation, stream sender and run_id.
    /// Returns None if the mission is already active.
    pub async fn register(&self, mission_id: &str) -> Option<MissionRegistration> {
        let mut missions = self.missions.write().await;
        if missions.contains_key(mission_id) {
            warn!("Mission already active, rejecting register: {}", mission_id);
            return None;
        }

        let token = CancellationToken::new();
        let (tx, _) = broadcast::channel(512);
        let now = std::time::Instant::now();
        let run_id = Uuid::new_v4().to_string();
        let mission = ActiveMission {
            mission_id: mission_id.to_string(),
            run_id: run_id.clone(),
            cancel_token: token.clone(),
            stream_tx: tx.clone(),
            buffer: Arc::new(Mutex::new(EventBuffer {
                next_id: 1,
                events: VecDeque::with_capacity(EVENT_HISTORY_LIMIT),
                last_activity: now,
            })),
            started_at: now,
            last_activity: Arc::new(Mutex::new(now)),
        };

        missions.insert(mission_id.to_string(), mission);
        info!("Mission registered: {} (run_id={})", mission_id, run_id);
        Some(MissionRegistration {
            cancel_token: token,
            stream_tx: tx,
            run_id,
        })
    }

    pub async fn active_run_id(&self, mission_id: &str) -> Option<String> {
        let missions = self.missions.read().await;
        missions.get(mission_id).map(|m| m.run_id.clone())
    }

    /// Subscribe to mission stream events
    pub async fn subscribe(
        &self,
        mission_id: &str,
    ) -> Option<broadcast::Receiver<MissionStreamEvent>> {
        let missions = self.missions.read().await;
        missions.get(mission_id).map(|m| m.stream_tx.subscribe())
    }

    /// Subscribe to mission stream and return recent buffered events newer than `after_id`.
    pub async fn subscribe_with_history(
        &self,
        mission_id: &str,
        after_id: Option<u64>,
    ) -> Option<(
        broadcast::Receiver<MissionStreamEvent>,
        Vec<MissionStreamEvent>,
    )> {
        let missions = self.missions.read().await;
        let mission = missions.get(mission_id)?;
        let rx = mission.stream_tx.subscribe();
        let history = mission.buffer.lock().ok().map_or_else(Vec::new, |b| {
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

    /// Broadcast an event to mission subscribers
    pub async fn broadcast(&self, mission_id: &str, event: StreamEvent) {
        let mut persist_item: Option<PersistMissionEvent> = None;
        {
            let missions = self.missions.read().await;
            if let Some(m) = missions.get(mission_id) {
                if let Ok(mut t) = m.last_activity.lock() {
                    *t = std::time::Instant::now();
                }
                if let Ok(mut buffer) = m.buffer.lock() {
                    buffer.last_activity = std::time::Instant::now();
                    let item = MissionStreamEvent {
                        id: buffer.next_id,
                        event: event.clone(),
                    };
                    buffer.next_id = buffer.next_id.saturating_add(1);
                    buffer.events.push_back(item.clone());
                    while buffer.events.len() > EVENT_HISTORY_LIMIT {
                        let _ = buffer.events.pop_front();
                    }
                    let _ = m.stream_tx.send(item.clone());
                    persist_item = Some(PersistMissionEvent {
                        mission_id: mission_id.to_string(),
                        run_id: m.run_id.clone(),
                        event_id: item.id,
                        event: item.event,
                    });
                } else {
                    // Fallback when lock is poisoned: still push event without id tracking.
                    let _ = m.stream_tx.send(MissionStreamEvent {
                        id: 0,
                        event: event.clone(),
                    });
                    persist_item = Some(PersistMissionEvent {
                        mission_id: mission_id.to_string(),
                        run_id: m.run_id.clone(),
                        event_id: 0,
                        event,
                    });
                }
            }
        }

        if let (Some(tx), Some(item)) = (&self.persist_tx, persist_item) {
            if let Err(e) = tx.send(item).await {
                warn!("Mission event persist queue closed: {}", e);
            }
        }
    }

    /// Mark mission as completed and remove from tracking
    pub async fn complete(&self, mission_id: &str) {
        let mut missions = self.missions.write().await;
        if let Some(m) = missions.remove(mission_id) {
            let duration = m.started_at.elapsed();
            info!(
                "Mission execution context closed: {} (duration: {:?})",
                mission_id, duration
            );
        }
    }

    /// Signal cancellation for an active mission without removing it from tracking.
    /// The executor cleanup path should call `complete()` after it exits.
    pub async fn signal_cancel(&self, mission_id: &str) -> bool {
        let missions = self.missions.read().await;
        if let Some(m) = missions.get(mission_id) {
            m.cancel_token.cancel();
            warn!("Mission cancellation signaled: {}", mission_id);
            true
        } else {
            false
        }
    }

    /// Force-cancel an active mission and remove from tracking immediately.
    /// Prefer `signal_cancel` in normal flows to avoid racing with executor cleanup.
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

    /// Return the currently active mission IDs tracked by this process.
    pub async fn active_mission_ids(&self) -> Vec<String> {
        let missions = self.missions.read().await;
        missions.keys().cloned().collect()
    }

    /// Clean up stale missions with no activity longer than max_age.
    /// Returns the removed mission IDs so caller can reconcile persisted state.
    pub async fn cleanup_stale(&self, max_age: std::time::Duration) -> Vec<String> {
        let mut missions = self.missions.write().await;
        let now = std::time::Instant::now();
        let mut removed_ids = Vec::new();
        missions.retain(|mid, m| {
            let last = m
                .last_activity
                .lock()
                .ok()
                .map(|t| *t)
                .unwrap_or(m.started_at);
            let inactive_for = now.saturating_duration_since(last);
            let stale = inactive_for > max_age;
            if stale {
                m.cancel_token.cancel();
                warn!(
                    "Cleaning up stale mission: {} (inactive_for: {:?})",
                    mid, inactive_for
                );
                removed_ids.push(mid.clone());
            }
            !stale
        });
        if !removed_ids.is_empty() {
            info!("Cleaned up {} stale missions", removed_ids.len());
        }
        removed_ids
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
