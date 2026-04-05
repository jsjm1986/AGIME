pub mod cfpm_extract_v2;
mod chat_history_search;
mod diagnostics;
pub mod extension_data;
mod legacy;
pub mod session_manager;

pub use diagnostics::generate_diagnostics;
pub use extension_data::{
    EnabledExtensionsState, ExtensionData, ExtensionState, ProjectedTaskRef, TaskItem,
    TaskProjectionEvent, TaskScope, TaskStatus, TasksStateV2, TodoState,
};
pub use session_manager::{Session, SessionInsights, SessionManager, SessionType, SharedSession};
