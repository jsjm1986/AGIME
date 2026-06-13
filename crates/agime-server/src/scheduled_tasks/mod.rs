//! Desktop scheduled tasks — natural language parsing, JSON-file persistence,
//! and background scheduling. Ports the team-server's scheduled-tasks module
//! with MongoDB swapped for atomic JSON files and team/channel/auth logic
//! stripped out.
//!
//! Layout:
//! - `data_dir()/scheduled_tasks/<task_id>.json`
//! - `data_dir()/scheduled_task_runs/<run_id>.json`

pub mod intent;
pub mod models;
pub mod routes;
pub mod scheduler;
pub mod service;
