//! Agent runtime policy resolver (team-server side).
//!
//! Most of the resolver — including the `resolve_document_policy`,
//! `builtin_registry_entry`, and `session_injected_capabilities` helpers — now
//! lives in [`agime_runtime::capability_policy`]. This module is a thin shim
//! that:
//!
//! * re-exports the runtime data shapes & helpers under the names team-server
//!   callers already use, and
//! * provides a team-side `AgentRuntimePolicyResolver` that accepts
//!   `&AgentSessionDoc` directly (as before) and forwards to the runtime
//!   resolver after converting the doc into a [`HostSessionPolicyContext`].
//!
//! Existing imports such as
//! `super::capability_policy::{AgentRuntimePolicyResolver, RuntimeCapabilitySnapshot, ...}`
//! keep working unchanged.

use std::collections::HashSet;

use agime_team::models::mongo::PortalEffectivePublicConfig;
use agime_team::models::TeamAgent;

pub use agime_runtime::capability_policy::{
    builtin_registry_entry, is_non_delegating_session_source, resolve_document_policy,
    source_delegation_override_for_session_source, HostSessionPolicyContext,
};
pub use agime_runtime::capability_types::{
    CapabilityKind, ConfiguredBuiltinCapability, DocumentScopeMode, DocumentWriteMode,
    ResolvedDocumentPolicy, RuntimeCapabilitySnapshot, RuntimeDelivery,
};

use super::session_mongo::AgentSessionDoc;

fn host_context_from_session(session: &AgentSessionDoc) -> HostSessionPolicyContext {
    HostSessionPolicyContext {
        session_source: session.session_source.clone(),
        portal_restricted: session.portal_restricted,
        document_access_mode: session.document_access_mode.clone(),
        document_scope_mode: session.document_scope_mode.clone(),
        document_write_mode: session.document_write_mode.clone(),
        allowed_extensions: session.allowed_extensions.clone(),
        allowed_skill_ids: session.allowed_skill_ids.clone(),
        delegation_policy_override: session.delegation_policy_override.clone(),
    }
}

pub struct AgentRuntimePolicyResolver;

impl AgentRuntimePolicyResolver {
    pub fn resolve(
        agent: &TeamAgent,
        session: Option<&AgentSessionDoc>,
        portal_effective: Option<&PortalEffectivePublicConfig>,
    ) -> RuntimeCapabilitySnapshot {
        Self::resolve_for_user_groups(agent, session, portal_effective, None)
    }

    pub fn resolve_for_user_groups(
        agent: &TeamAgent,
        session: Option<&AgentSessionDoc>,
        portal_effective: Option<&PortalEffectivePublicConfig>,
        user_group_ids: Option<&HashSet<String>>,
    ) -> RuntimeCapabilitySnapshot {
        let context = session.map(host_context_from_session);
        agime_runtime::capability_policy::AgentRuntimePolicyResolver::resolve_for_user_groups(
            agent,
            context.as_ref(),
            portal_effective,
            user_group_ids,
        )
    }
}
