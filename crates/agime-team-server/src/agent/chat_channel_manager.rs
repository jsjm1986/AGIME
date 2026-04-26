use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use uuid::Uuid;

use super::chat_channels::ChatChannelService;
use super::task_manager::StreamEvent;
use agime_team::MongoDb;

const EVENT_HISTORY_LIMIT: usize = 400;
const EVENT_PERSIST_BATCH_SIZE: usize = 128;
const EVENT_PERSIST_FLUSH_MS: u64 = 25;
const EVENT_PERSIST_QUEUE_CAP: usize = 8192;

#[derive(Clone, Debug)]
pub struct ChannelStreamEvent {
    pub id: u64,
    pub event: StreamEvent,
    pub thread_root_id: Option<String>,
    pub root_message_id: Option<String>,
}

struct EventBuffer {
    next_id: u64,
    events: VecDeque<ChannelStreamEvent>,
    last_activity: std::time::Instant,
}

#[derive(Clone, Debug)]
struct PersistChannelEvent {
    channel_id: String,
    run_id: String,
    thread_root_id: Option<String>,
    root_message_id: Option<String>,
    event_id: u64,
    event: StreamEvent,
}

pub struct ActiveChannelRun {
    pub cancel_token: CancellationToken,
    pub run_id: String,
    pub thread_root_id: Option<String>,
    pub root_message_id: Option<String>,
    pub started_at: std::time::Instant,
}

struct ChannelRunStream {
    stream_tx: broadcast::Sender<ChannelStreamEvent>,
    buffer: Arc<Mutex<EventBuffer>>,
    runs: HashMap<String, ActiveChannelRun>,
    latest_run_id: Option<String>,
}

pub struct ChatChannelManager {
    runs: RwLock<HashMap<String, ChannelRunStream>>,
    persist_tx: mpsc::Sender<PersistChannelEvent>,
}

impl ChatChannelManager {
    fn parse_context_id(context_id: &str) -> (&str, Option<&str>) {
        context_id
            .split_once("::")
            .map(|(channel_id, run_scope_id)| (channel_id, Some(run_scope_id)))
            .unwrap_or((context_id, None))
    }

    fn default_run_scope_id() -> String {
        "__channel_root__".to_string()
    }

    pub fn new_with_event_persistence(db: Arc<MongoDb>) -> Self {
        let (tx, mut rx) = mpsc::channel::<PersistChannelEvent>(EVENT_PERSIST_QUEUE_CAP);
        tokio::spawn(async move {
            let service = ChatChannelService::new(db);
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
                let grouped = batch.into_iter().fold(
                    HashMap::<
                        (String, Option<String>, Option<String>),
                        Vec<(String, u64, StreamEvent)>,
                    >::new(),
                    |mut acc, item| {
                        acc.entry((
                            item.channel_id,
                            item.thread_root_id.clone(),
                            item.root_message_id.clone(),
                        ))
                        .or_default()
                        .push((item.run_id, item.event_id, item.event));
                        acc
                    },
                );
                for ((channel_id, thread_root_id, root_message_id), records) in grouped {
                    if let Err(error) = service
                        .save_channel_events(
                            &channel_id,
                            thread_root_id.as_deref(),
                            root_message_id.as_deref(),
                            &records,
                        )
                        .await
                    {
                        warn!(
                            "Failed to persist channel stream events (channel={}, count={}): {}",
                            channel_id,
                            records.len(),
                            error
                        );
                    }
                }
            }
            info!("Chat channel event persistence worker stopped");
        });

        Self {
            runs: RwLock::new(HashMap::new()),
            persist_tx: tx,
        }
    }

    pub async fn register(
        &self,
        channel_id: &str,
        run_scope_id: String,
        thread_root_id: Option<String>,
        root_message_id: Option<String>,
    ) -> Option<(
        CancellationToken,
        broadcast::Sender<ChannelStreamEvent>,
        String,
    )> {
        let mut runs = self.runs.write().await;
        let channel_runs = runs.entry(channel_id.to_string()).or_insert_with(|| {
            let (tx, _) = broadcast::channel(512);
            let now = std::time::Instant::now();
            ChannelRunStream {
                stream_tx: tx,
                buffer: Arc::new(Mutex::new(EventBuffer {
                    next_id: 1,
                    events: VecDeque::with_capacity(EVENT_HISTORY_LIMIT),
                    last_activity: now,
                })),
                runs: HashMap::new(),
                latest_run_id: None,
            }
        });
        if channel_runs.runs.contains_key(&run_scope_id) {
            warn!(
                "Channel thread already active, rejecting register: channel={} scope={}",
                channel_id, run_scope_id
            );
            return None;
        }

        let token = CancellationToken::new();
        let now = std::time::Instant::now();
        let run_id = Uuid::new_v4().to_string();
        let active = ActiveChannelRun {
            cancel_token: token.clone(),
            run_id: run_id.clone(),
            thread_root_id,
            root_message_id,
            started_at: now,
        };
        channel_runs.latest_run_id = Some(run_id.clone());
        channel_runs.runs.insert(run_scope_id, active);
        Some((token, channel_runs.stream_tx.clone(), run_id))
    }

    pub async fn active_run_id(&self, channel_id: &str) -> Option<String> {
        let runs = self.runs.read().await;
        runs.get(channel_id)
            .and_then(|channel| channel.latest_run_id.clone())
    }

    pub async fn subscribe_with_history(
        &self,
        channel_id: &str,
        after_id: Option<u64>,
    ) -> Option<(
        broadcast::Receiver<ChannelStreamEvent>,
        Vec<ChannelStreamEvent>,
    )> {
        let runs = self.runs.read().await;
        let channel = runs.get(channel_id)?;
        let rx = channel.stream_tx.subscribe();
        let history = channel.buffer.lock().ok().map_or_else(Vec::new, |buffer| {
            if let Some(after) = after_id {
                buffer
                    .events
                    .iter()
                    .filter(|item| item.id > after)
                    .cloned()
                    .collect()
            } else {
                buffer.events.iter().cloned().collect()
            }
        });
        Some((rx, history))
    }

    pub async fn broadcast(&self, channel_id: &str, event: StreamEvent) {
        let (channel_id, run_scope_id) = Self::parse_context_id(channel_id);
        let mut persist: Option<PersistChannelEvent> = None;
        {
            let runs = self.runs.read().await;
            if let Some(channel) = runs.get(channel_id) {
                if let Ok(mut buffer) = channel.buffer.lock() {
                    buffer.last_activity = std::time::Instant::now();
                    let active_run = run_scope_id
                        .and_then(|scope| channel.runs.get(scope))
                        .or_else(|| {
                            if channel.runs.len() == 1 {
                                channel.runs.values().next()
                            } else {
                                None
                            }
                        });
                    let item = ChannelStreamEvent {
                        id: buffer.next_id,
                        event: event.clone(),
                        thread_root_id: active_run.and_then(|run| run.thread_root_id.clone()),
                        root_message_id: active_run.and_then(|run| run.root_message_id.clone()),
                    };
                    buffer.next_id = buffer.next_id.saturating_add(1);
                    buffer.events.push_back(item.clone());
                    while buffer.events.len() > EVENT_HISTORY_LIMIT {
                        let _ = buffer.events.pop_front();
                    }
                    let _ = channel.stream_tx.send(item.clone());
                    if let Some(run) = active_run {
                        persist = Some(PersistChannelEvent {
                            channel_id: channel_id.to_string(),
                            run_id: run.run_id.clone(),
                            thread_root_id: run.thread_root_id.clone(),
                            root_message_id: run.root_message_id.clone(),
                            event_id: item.id,
                            event: item.event,
                        });
                    }
                }
            }
        }
        if let Some(item) = persist {
            let _ = self.persist_tx.send(item).await;
        }
    }

    pub async fn complete(&self, channel_id: &str, run_scope_id: Option<&str>) {
        let mut runs = self.runs.write().await;
        if let Some(channel) = runs.get_mut(channel_id) {
            let scope_id = run_scope_id
                .map(str::to_string)
                .unwrap_or_else(Self::default_run_scope_id);
            if let Some(run) = channel.runs.remove(&scope_id) {
                info!(
                    "Chat channel run completed: {} scope={} ({:?})",
                    channel_id,
                    scope_id,
                    run.started_at.elapsed()
                );
            }
            if channel.runs.is_empty() {
                runs.remove(channel_id);
            }
        }
    }

    pub async fn unregister(&self, channel_id: &str, run_scope_id: Option<&str>) {
        let mut runs = self.runs.write().await;
        if let Some(channel) = runs.get_mut(channel_id) {
            let scope_id = run_scope_id
                .map(str::to_string)
                .unwrap_or_else(Self::default_run_scope_id);
            if channel.runs.remove(&scope_id).is_some() {
                warn!(
                    "Chat channel run unregistered: {} scope={}",
                    channel_id, scope_id
                );
            }
            if channel.runs.is_empty() {
                runs.remove(channel_id);
            }
        }
    }

    pub async fn cancel(&self, channel_id: &str, run_scope_id: Option<&str>) -> bool {
        let mut runs = self.runs.write().await;
        if let Some(channel) = runs.get_mut(channel_id) {
            let scope_id = run_scope_id
                .map(str::to_string)
                .or_else(|| channel.runs.keys().next().cloned())
                .unwrap_or_else(Self::default_run_scope_id);
            if let Some(run) = channel.runs.remove(&scope_id) {
                run.cancel_token.cancel();
                if channel.runs.is_empty() {
                    runs.remove(channel_id);
                }
                return true;
            }
        }
        false
    }
}

impl super::runtime::EventBroadcaster for ChatChannelManager {
    async fn broadcast(&self, context_id: &str, event: StreamEvent) {
        self.broadcast(context_id, event).await;
    }
}
