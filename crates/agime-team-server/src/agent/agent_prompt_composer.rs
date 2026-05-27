//! Agent prompt composer (team-server side).
//!
//! The actual composer lives in [`agime_runtime::prompt_composer`]; this file
//! re-exports the symbols team-server callers consume so existing
//! `super::agent_prompt_composer::*` imports keep working unchanged.

pub use agime_runtime::prompt_composer::{
    build_prompt_introspection_snapshot, compose_top_level_prompt, AgentPromptComposerInput,
};
