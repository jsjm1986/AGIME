// Extension data management for sessions
// Provides a simple way to store extension-specific data with versioned keys

use crate::config::ExtensionConfig;
use anyhow::Result;
use chrono::Utc;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use utoipa::ToSchema;

/// Extension data containing all extension states
/// Keys are in format "extension_name.version" (e.g., "todo.v0")
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct ExtensionData {
    #[serde(flatten)]
    pub extension_states: HashMap<String, Value>,
}

impl ExtensionData {
    /// Create a new empty ExtensionData
    pub fn new() -> Self {
        Self {
            extension_states: HashMap::new(),
        }
    }

    /// Get extension state for a specific extension and version
    pub fn get_extension_state(&self, extension_name: &str, version: &str) -> Option<&Value> {
        let key = format!("{}.{}", extension_name, version);
        self.extension_states.get(&key)
    }

    /// Set extension state for a specific extension and version
    pub fn set_extension_state(&mut self, extension_name: &str, version: &str, state: Value) {
        let key = format!("{}.{}", extension_name, version);
        self.extension_states.insert(key, state);
    }
}

/// Helper trait for extension-specific state management
pub trait ExtensionState: Sized + Serialize + for<'de> Deserialize<'de> {
    /// The name of the extension
    const EXTENSION_NAME: &'static str;

    /// The version of the extension state format
    const VERSION: &'static str;

    /// Convert from JSON value
    fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            anyhow::anyhow!(
                "Failed to deserialize {} state: {}",
                Self::EXTENSION_NAME,
                e
            )
        })
    }

    /// Convert to JSON value
    fn to_value(&self) -> Result<Value> {
        serde_json::to_value(self).map_err(|e| {
            anyhow::anyhow!("Failed to serialize {} state: {}", Self::EXTENSION_NAME, e)
        })
    }

    /// Get state from extension data
    fn from_extension_data(extension_data: &ExtensionData) -> Option<Self> {
        extension_data
            .get_extension_state(Self::EXTENSION_NAME, Self::VERSION)
            .and_then(|v| Self::from_value(v).ok())
    }

    /// Save state to extension data
    fn to_extension_data(&self, extension_data: &mut ExtensionData) -> Result<()> {
        let value = self.to_value()?;
        extension_data.set_extension_state(Self::EXTENSION_NAME, Self::VERSION, value);
        Ok(())
    }
}

/// TODO extension state implementation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoState {
    pub content: String,
}

impl ExtensionState for TodoState {
    const EXTENSION_NAME: &'static str = "todo";
    const VERSION: &'static str = "v0";
}

impl TodoState {
    /// Create a new TODO state
    pub fn new(content: String) -> Self {
        Self { content }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskScope {
    Leader,
    Worker,
    Standalone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskItem {
    pub id: String,
    pub subject: String,
    pub description: String,
    pub active_form: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectedTaskRef {
    pub leader_task_id: String,
    pub source_task_id: String,
    pub source_session_id: String,
    pub logical_worker_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TaskProjectionEvent {
    pub board_id: String,
    pub source_scope: TaskScope,
    pub source_session_id: String,
    #[serde(default)]
    pub projected_tasks: Vec<ProjectedTaskRef>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TasksStateV2 {
    pub scope: TaskScope,
    pub board_id: String,
    pub high_water_mark: u64,
    #[serde(default)]
    pub items: Vec<TaskItem>,
    pub updated_at: String,
}

impl ExtensionState for TasksStateV2 {
    const EXTENSION_NAME: &'static str = "tasks";
    const VERSION: &'static str = "v2";
}

impl TasksStateV2 {
    pub fn new(scope: TaskScope, board_id: impl Into<String>) -> Self {
        Self {
            scope,
            board_id: board_id.into(),
            high_water_mark: 0,
            items: Vec::new(),
            updated_at: Utc::now().to_rfc3339(),
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now().to_rfc3339();
    }

    pub fn next_task_id(&mut self) -> String {
        self.high_water_mark = self.high_water_mark.saturating_add(1);
        self.touch();
        self.high_water_mark.to_string()
    }

    pub fn get_task(&self, task_id: &str) -> Option<&TaskItem> {
        self.items.iter().find(|item| item.id == task_id)
    }

    pub fn get_task_mut(&mut self, task_id: &str) -> Option<&mut TaskItem> {
        self.items.iter_mut().find(|item| item.id == task_id)
    }

    pub fn migrate_from_legacy_todo(
        legacy: &TodoState,
        scope: TaskScope,
        board_id: impl Into<String>,
    ) -> Self {
        let mut state = Self::new(scope, board_id);
        let mut parsed_items = parse_legacy_todo_items(&legacy.content);
        if parsed_items.is_empty() && !legacy.content.trim().is_empty() {
            let id = state.next_task_id();
            parsed_items.push(TaskItem {
                id,
                subject: "Imported legacy todo".to_string(),
                description: "Legacy TODO content imported into Tasks V2".to_string(),
                active_form: "Importing legacy todo".to_string(),
                owner: None,
                status: TaskStatus::Pending,
                blocks: Vec::new(),
                blocked_by: Vec::new(),
                metadata: HashMap::from([(
                    "raw_legacy_todo".to_string(),
                    Value::String(legacy.content.clone()),
                )]),
            });
        }
        state.high_water_mark = parsed_items
            .iter()
            .filter_map(|item| item.id.parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        state.items = parsed_items;
        state.touch();
        state
    }
}

fn parse_legacy_todo_items(content: &str) -> Vec<TaskItem> {
    let mut items = Vec::new();
    let mut next_id = 1_u64;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let (status, subject) = if let Some(rest) = line
            .strip_prefix("- [ ] ")
            .or_else(|| line.strip_prefix("* [ ] "))
        {
            (TaskStatus::Pending, rest.trim())
        } else if let Some(rest) = line
            .strip_prefix("- [x] ")
            .or_else(|| line.strip_prefix("- [X] "))
            .or_else(|| line.strip_prefix("* [x] "))
            .or_else(|| line.strip_prefix("* [X] "))
        {
            (TaskStatus::Completed, rest.trim())
        } else if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            (TaskStatus::Pending, rest.trim())
        } else if let Some((_, rest)) = line.split_once(". ") {
            (TaskStatus::Pending, rest.trim())
        } else {
            continue;
        };

        if subject.is_empty() {
            continue;
        }

        let subject = subject.to_string();
        items.push(TaskItem {
            id: next_id.to_string(),
            description: subject.clone(),
            active_form: format!("Working on {}", subject),
            subject,
            owner: None,
            status,
            blocks: Vec::new(),
            blocked_by: Vec::new(),
            metadata: HashMap::new(),
        });
        next_id = next_id.saturating_add(1);
    }
    items
}

/// Enabled extensions state implementation for storing which extensions are active
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnabledExtensionsState {
    pub extensions: Vec<ExtensionConfig>,
}

impl ExtensionState for EnabledExtensionsState {
    const EXTENSION_NAME: &'static str = "enabled_extensions";
    const VERSION: &'static str = "v0";
}

impl EnabledExtensionsState {
    pub fn new(extensions: Vec<ExtensionConfig>) -> Self {
        Self { extensions }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extension_data_basic_operations() {
        let mut extension_data = ExtensionData::new();

        // Test setting and getting extension state
        let todo_state = json!({"content": "- Task 1\n- Task 2"});
        extension_data.set_extension_state("todo", "v0", todo_state.clone());

        assert_eq!(
            extension_data.get_extension_state("todo", "v0"),
            Some(&todo_state)
        );
        assert_eq!(extension_data.get_extension_state("todo", "v1"), None);
    }

    #[test]
    fn test_multiple_extension_states() {
        let mut extension_data = ExtensionData::new();

        // Add multiple extension states
        extension_data.set_extension_state("todo", "v0", json!("TODO content"));
        extension_data.set_extension_state("memory", "v1", json!({"items": ["item1", "item2"]}));
        extension_data.set_extension_state("config", "v2", json!({"setting": true}));

        // Check all states exist
        assert_eq!(extension_data.extension_states.len(), 3);
        assert!(extension_data.get_extension_state("todo", "v0").is_some());
        assert!(extension_data.get_extension_state("memory", "v1").is_some());
        assert!(extension_data.get_extension_state("config", "v2").is_some());
    }

    #[test]
    fn test_todo_state_trait() {
        let mut extension_data = ExtensionData::new();

        // Create and save TODO state
        let todo = TodoState::new("- Task 1\n- Task 2".to_string());
        todo.to_extension_data(&mut extension_data).unwrap();

        // Retrieve TODO state
        let retrieved = TodoState::from_extension_data(&extension_data);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().content, "- Task 1\n- Task 2");
    }

    #[test]
    fn test_extension_data_serialization() {
        let mut extension_data = ExtensionData::new();
        extension_data.set_extension_state("todo", "v0", json!("TODO content"));
        extension_data.set_extension_state("memory", "v1", json!({"key": "value"}));

        // Serialize to JSON
        let json = serde_json::to_value(&extension_data).unwrap();

        // Check the structure
        assert!(json.is_object());
        assert_eq!(json.get("todo.v0"), Some(&json!("TODO content")));
        assert_eq!(json.get("memory.v1"), Some(&json!({"key": "value"})));

        // Deserialize back
        let deserialized: ExtensionData = serde_json::from_value(json).unwrap();
        assert_eq!(
            deserialized.get_extension_state("todo", "v0"),
            Some(&json!("TODO content"))
        );
        assert_eq!(
            deserialized.get_extension_state("memory", "v1"),
            Some(&json!({"key": "value"}))
        );
    }

    #[test]
    fn migrate_legacy_checkbox_todo_into_tasks_v2() {
        let legacy = TodoState::new("- [ ] implement task system\n- [x] write tests".to_string());
        let migrated =
            TasksStateV2::migrate_from_legacy_todo(&legacy, TaskScope::Standalone, "session-1");

        assert_eq!(migrated.board_id, "session-1");
        assert_eq!(migrated.items.len(), 2);
        assert_eq!(migrated.items[0].subject, "implement task system");
        assert_eq!(migrated.items[0].status, TaskStatus::Pending);
        assert_eq!(migrated.items[1].subject, "write tests");
        assert_eq!(migrated.items[1].status, TaskStatus::Completed);
    }

    #[test]
    fn migrate_unstructured_legacy_todo_creates_import_task() {
        let legacy = TodoState::new("need to figure this out later".to_string());
        let migrated =
            TasksStateV2::migrate_from_legacy_todo(&legacy, TaskScope::Standalone, "session-2");

        assert_eq!(migrated.items.len(), 1);
        assert_eq!(migrated.items[0].subject, "Imported legacy todo");
        assert_eq!(
            migrated.items[0]
                .metadata
                .get("raw_legacy_todo")
                .and_then(Value::as_str),
            Some("need to figure this out later")
        );
    }
}
