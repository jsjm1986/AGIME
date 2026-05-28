//! Desktop harness host — Phase 3: capability policy facade.
//!
//! The team-server's `capability_policy.rs` (1190 lines, lives in
//! `crates/agime-runtime/src/capability_policy.rs`) is tightly coupled to
//! `agime_team::models::*` (Mongo / portal / skill-binding types). The desktop
//! has none of those concepts: no portals, no document-scoped session
//! sources, no skill bindings. Copying the full file would force a hard
//! dependency on `agime_team` which violates the dual-track constraint.
//!
//! Instead this module provides the **same call surface** (`HostSessionPolicyContext`,
//! resolver entry points) but with a desktop-flavored data model:
//! - `ApprovalMode` enum mirrors the desktop UI's permission settings
//! - `RuntimeCapabilitySnapshot` carries only what the desktop reply path
//!   actually consumes — allowed extensions / approval mode / minor flags
//!
//! Future phases that want to inject capability decisions before tool calls
//! can call `resolve_capabilities(...)` and inspect the snapshot. The default
//! resolver is permissive (no extra restrictions), preserving today's desktop
//! behavior bit-for-bit when callers don't supply a policy context.
//!
//! SOURCE: crates/agime-runtime/src/capability_policy.rs at commit 961109f
//! (desktop reimplementation, not a verbatim copy — see CLAUDE.md long-term
//! maintenance strategy).

#![cfg(feature = "desktop_harness_host")]

use std::collections::HashSet;

/// Approval mode mirrors the desktop UI's three-state permission setting:
/// - `Auto`: tools execute without prompting
/// - `Approve`: every tool call surfaces an approval request
/// - `Manual`: only flagged tools surface; rest auto-execute
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalMode {
    Auto,
    Approve,
    Manual,
}

impl Default for ApprovalMode {
    fn default() -> Self {
        ApprovalMode::Auto
    }
}

impl ApprovalMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalMode::Auto => "auto",
            ApprovalMode::Approve => "approve",
            ApprovalMode::Manual => "manual",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Some(ApprovalMode::Auto),
            "approve" | "always" => Some(ApprovalMode::Approve),
            "manual" | "smart_approve" => Some(ApprovalMode::Manual),
            _ => None,
        }
    }
}

/// Session-derived inputs to the resolver. Mirror of the team-server's
/// `HostSessionPolicyContext` shape but stripped of portal / skill / document
/// fields the desktop has no concept of.
#[derive(Debug, Clone, Default)]
pub struct HostSessionPolicyContext {
    pub session_source: String,
    pub allowed_extensions: Option<Vec<String>>,
    pub manual_approval_tools: Option<Vec<String>>,
    pub approval_mode: ApprovalMode,
}

/// Capability snapshot consumed by the desktop harness host during a turn.
/// The team-server snapshot has 30+ fields; the desktop trims to the minimum
/// the reply path actually checks.
#[derive(Debug, Clone, Default)]
pub struct RuntimeCapabilitySnapshot {
    #[allow(dead_code)]
    pub session_source: String,
    pub approval_mode: ApprovalMode,
    pub allowed_extensions: Option<HashSet<String>>,
    pub manual_approval_tools: HashSet<String>,
}

impl RuntimeCapabilitySnapshot {
    /// Returns `true` if this tool call must surface an approval request to
    /// the user. Default `Auto` mode never requests approval; `Approve` mode
    /// always does; `Manual` only requests for tools flagged by the session.
    pub fn requires_approval(&self, tool_name: &str) -> bool {
        match self.approval_mode {
            ApprovalMode::Auto => false,
            ApprovalMode::Approve => true,
            ApprovalMode::Manual => self
                .manual_approval_tools
                .iter()
                .any(|name| name.as_str() == tool_name),
        }
    }

    /// Returns `true` if the named extension is allowed in this session.
    /// `None` means "no allow-list configured" (permissive — same as today).
    pub fn extension_allowed(&self, runtime_name: &str) -> bool {
        match &self.allowed_extensions {
            None => true,
            Some(set) => set.contains(runtime_name),
        }
    }
}

/// Resolve a snapshot from a session policy context. Pure function — no I/O,
/// no Mongo. Matches the team-server entry shape so future phases can swap
/// in a richer resolver without touching call sites.
pub fn resolve_capabilities(ctx: &HostSessionPolicyContext) -> RuntimeCapabilitySnapshot {
    RuntimeCapabilitySnapshot {
        session_source: ctx.session_source.clone(),
        approval_mode: ctx.approval_mode,
        allowed_extensions: ctx
            .allowed_extensions
            .as_ref()
            .map(|list| list.iter().cloned().collect()),
        manual_approval_tools: ctx
            .manual_approval_tools
            .as_ref()
            .map(|list| list.iter().cloned().collect())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_mode_skips_approval() {
        let snap = resolve_capabilities(&HostSessionPolicyContext::default());
        assert!(!snap.requires_approval("developer__shell"));
    }

    #[test]
    fn approve_mode_always_requests() {
        let ctx = HostSessionPolicyContext {
            approval_mode: ApprovalMode::Approve,
            ..Default::default()
        };
        let snap = resolve_capabilities(&ctx);
        assert!(snap.requires_approval("anything"));
    }

    #[test]
    fn manual_mode_uses_allow_list() {
        let ctx = HostSessionPolicyContext {
            approval_mode: ApprovalMode::Manual,
            manual_approval_tools: Some(vec!["developer__shell".to_string()]),
            ..Default::default()
        };
        let snap = resolve_capabilities(&ctx);
        assert!(snap.requires_approval("developer__shell"));
        assert!(!snap.requires_approval("memory__remember"));
    }

    #[test]
    fn extension_allow_list_none_is_permissive() {
        let snap = resolve_capabilities(&HostSessionPolicyContext::default());
        assert!(snap.extension_allowed("anything"));
    }

    #[test]
    fn extension_allow_list_some_is_restrictive() {
        let ctx = HostSessionPolicyContext {
            allowed_extensions: Some(vec!["developer".to_string()]),
            ..Default::default()
        };
        let snap = resolve_capabilities(&ctx);
        assert!(snap.extension_allowed("developer"));
        assert!(!snap.extension_allowed("memory"));
    }

    #[test]
    fn approval_mode_round_trip() {
        for mode in [
            ApprovalMode::Auto,
            ApprovalMode::Approve,
            ApprovalMode::Manual,
        ] {
            assert_eq!(ApprovalMode::from_str(mode.as_str()), Some(mode));
        }
    }
}
