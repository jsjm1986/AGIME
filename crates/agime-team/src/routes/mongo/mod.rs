//! MongoDB routes module

pub mod audit;
pub mod documents;
pub mod extensions;
pub mod folders;
pub mod recipes;
pub mod skills;
pub mod smart_log;
pub mod stats;
pub mod sync;
pub mod teams;
pub mod unified;
pub mod user_groups;
pub mod portals;

use axum::Router;
use serde::Serialize;
use std::sync::Arc;

pub use teams::AppState;

/// Shared install/uninstall response used by skills, recipes, and extensions
#[derive(Debug, Serialize)]
pub struct InstallResponse {
    pub success: bool,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "resourceId")]
    pub resource_id: String,
    #[serde(rename = "localPath")]
    pub local_path: Option<String>,
    pub error: Option<String>,
}

/// Configure all MongoDB routes
pub fn configure(state: Arc<AppState>) -> Router {
    Router::new()
        .merge(teams::team_routes())
        .merge(documents::document_routes())
        .merge(skills::skill_routes())
        .merge(recipes::recipe_routes())
        .merge(extensions::extension_routes())
        .merge(folders::folder_routes())
        .merge(audit::audit_routes())
        .merge(stats::stats_routes())
        .merge(sync::sync_routes())
        .merge(user_groups::user_group_routes())
        .merge(unified::unified_routes())
        .merge(smart_log::smart_log_routes())
        .merge(portals::portal_routes())
        .with_state(state)
}
