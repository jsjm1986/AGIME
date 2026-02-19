//! AGIME Team Collaboration Module
//!
//! This module provides team collaboration functionality for sharing
//! Skills, Recipes, and Extensions between team members.
//!
//! # Features
//! - Team management (create, update, delete teams)
//! - Member management with role-based permissions
//! - Resource sharing (Skills, Recipes, Extensions)
//! - Resource installation and version tracking
//! - Git-based synchronization
//! - MCP tools for AI integration
//! - MongoDB support with GridFS for documents

pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod mcp;
pub mod models;
pub mod routes;
pub mod security;
pub mod services;
pub mod sync;

pub use config::TeamConfig;
pub use db::MongoDb;
pub use error::{TeamError, TeamResult};

/// Authenticated user ID from auth middleware
#[derive(Clone, Debug)]
pub struct AuthenticatedUserId(pub String);

impl AuthenticatedUserId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
