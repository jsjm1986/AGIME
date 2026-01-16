//! Error types for the team module

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Result type alias for team operations
pub type TeamResult<T> = Result<T, TeamError>;

/// Team module error types
#[derive(Debug, Error)]
pub enum TeamError {
    #[error("Team not found: {0}")]
    TeamNotFound(String),

    #[error("Member not found: {0}")]
    MemberNotFound(String),

    #[error("Invite not found: {0}")]
    InviteNotFound(String),

    #[error("Resource not found: {resource_type} {resource_id}")]
    ResourceNotFound {
        resource_type: String,
        resource_id: String,
    },

    #[error("Permission denied: {action}")]
    PermissionDenied { action: String },

    #[error("Resource already exists: {name} v{version}")]
    ResourceExists { name: String, version: String },

    #[error("Team already exists: {name}")]
    TeamExists { name: String },

    #[error("Member already exists in team: {user_id}")]
    MemberExists { user_id: String },

    #[error("Invalid resource content: {reason}")]
    InvalidContent { reason: String },

    #[error("Invalid resource name: {name} (contains forbidden characters or path sequences)")]
    InvalidResourceName { name: String },

    #[error("Invalid version format: {version}")]
    InvalidVersion { version: String },

    #[error("Dependency not found: {dep_type} {dep_name}")]
    DependencyNotFound { dep_type: String, dep_name: String },

    #[error("Circular dependency detected: {path}")]
    CircularDependency { path: String },

    #[error("Cannot remove team owner")]
    CannotRemoveOwner,

    #[error("Owner cannot leave team")]
    OwnerCannotLeave,

    #[error("Sync failed: {reason}")]
    SyncFailed { reason: String },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("IO error: {0}")]
    Io(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<sqlx::Error> for TeamError {
    fn from(err: sqlx::Error) -> Self {
        TeamError::Database(err.to_string())
    }
}

impl From<std::io::Error> for TeamError {
    fn from(err: std::io::Error) -> Self {
        TeamError::Io(err.to_string())
    }
}

impl From<serde_json::Error> for TeamError {
    fn from(err: serde_json::Error) -> Self {
        TeamError::Serialization(err.to_string())
    }
}

/// API error response
#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl TeamError {
    /// Convert to API error code
    pub fn code(&self) -> &'static str {
        match self {
            TeamError::TeamNotFound(_) => "TEAM_NOT_FOUND",
            TeamError::MemberNotFound(_) => "MEMBER_NOT_FOUND",
            TeamError::InviteNotFound(_) => "INVITE_NOT_FOUND",
            TeamError::ResourceNotFound { .. } => "RESOURCE_NOT_FOUND",
            TeamError::PermissionDenied { .. } => "PERMISSION_DENIED",
            TeamError::ResourceExists { .. } => "RESOURCE_EXISTS",
            TeamError::TeamExists { .. } => "TEAM_EXISTS",
            TeamError::MemberExists { .. } => "MEMBER_EXISTS",
            TeamError::InvalidContent { .. } => "INVALID_CONTENT",
            TeamError::InvalidResourceName { .. } => "INVALID_RESOURCE_NAME",
            TeamError::InvalidVersion { .. } => "INVALID_VERSION",
            TeamError::DependencyNotFound { .. } => "DEPENDENCY_NOT_FOUND",
            TeamError::CircularDependency { .. } => "CIRCULAR_DEPENDENCY",
            TeamError::CannotRemoveOwner => "CANNOT_REMOVE_OWNER",
            TeamError::OwnerCannotLeave => "OWNER_CANNOT_LEAVE",
            TeamError::SyncFailed { .. } => "SYNC_FAILED",
            TeamError::Validation(_) => "VALIDATION_ERROR",
            TeamError::Database(_) => "DATABASE_ERROR",
            TeamError::Io(_) => "IO_ERROR",
            TeamError::Serialization(_) => "SERIALIZATION_ERROR",
            TeamError::Internal(_) => "INTERNAL_ERROR",
        }
    }

    /// Get HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            TeamError::TeamNotFound(_)
            | TeamError::MemberNotFound(_)
            | TeamError::InviteNotFound(_)
            | TeamError::ResourceNotFound { .. } => StatusCode::NOT_FOUND,

            TeamError::PermissionDenied { .. } => StatusCode::FORBIDDEN,

            TeamError::ResourceExists { .. }
            | TeamError::TeamExists { .. }
            | TeamError::MemberExists { .. } => StatusCode::CONFLICT,

            TeamError::InvalidContent { .. }
            | TeamError::InvalidResourceName { .. }
            | TeamError::InvalidVersion { .. }
            | TeamError::Validation(_)
            | TeamError::CircularDependency { .. } => StatusCode::BAD_REQUEST,

            TeamError::CannotRemoveOwner | TeamError::OwnerCannotLeave => StatusCode::BAD_REQUEST,

            TeamError::DependencyNotFound { .. } => StatusCode::UNPROCESSABLE_ENTITY,

            TeamError::SyncFailed { .. }
            | TeamError::Database(_)
            | TeamError::Io(_)
            | TeamError::Serialization(_)
            | TeamError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for TeamError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = ApiError {
            code: self.code().to_string(),
            message: self.to_string(),
            details: None,
        };

        (status, axum::Json(body)).into_response()
    }
}
