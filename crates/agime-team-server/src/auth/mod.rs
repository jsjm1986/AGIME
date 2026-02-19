//! Authentication module for the team server
//!
//! Supports both MongoDB and SQLite auth implementations.

pub mod api_key;

// SQLite versions
#[path = "middleware.rs"]
pub mod middleware_sqlite;
#[path = "routes.rs"]
pub mod routes_sqlite;
#[path = "service.rs"]
pub mod service_sqlite;
#[path = "session.rs"]
pub mod session_sqlite;

// MongoDB versions (active)
pub mod middleware_mongo;
pub mod routes_mongo;
pub mod service_mongo;
pub mod session_mongo;

// Backward-compatible re-exports (default to Mongo).
pub use middleware_mongo as middleware;
pub use routes_mongo as routes;
pub use service_mongo as service;
pub use session_mongo as session;

// api_key functions are used internally by services
