//! MCP module - Model Context Protocol tools

pub mod team_client;
pub mod team_tools;

pub use team_client::*;
pub use team_tools::*;

/// Team extension name constant
pub const TEAM_EXTENSION_NAME: &str = "team";
