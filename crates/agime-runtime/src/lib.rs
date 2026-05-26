//! agime-runtime — shared harness wrapper layer.
//!
//! This crate hosts the runtime "shell" code that surrounds the core
//! [`agime::agents::harness`] turn loop and is shared by both:
//!
//! * the desktop server (`agime-server`)
//! * the team server (`agime-team-server`)
//!
//! Concrete persistence (Mongo / filesystem), broadcasters (team SSE / desktop
//! SSE), task queues etc. are injected by each side via the traits in
//! [`crate::traits`].
//!
//! The first iteration only exposes the tool-dispatch helper used by the
//! provider turn while a tool is being invoked; further harness machinery is
//! migrated incrementally — see the migration plan for the full extraction
//! roadmap.

pub mod admission;
pub mod capability_policy;
pub mod capability_types;
pub mod execution_admission;
pub mod prompt_composer;
pub mod provider_factory;
pub mod tool_content;
pub mod tool_dispatch;
pub mod traits;

pub use admission::{wait_for_execution_slot, ExecutionSlotAcquireOutcome, ExecutionSlotProvider};
pub use execution_admission::{
    admit_or_queue_task, resume_queued_tasks, start_next_queued_tasks_for_agent,
    QueuedTaskBroadcaster, QueuedTaskRef, TaskAdmissionOutcome, TaskQueueRepository,
    TaskRuntimeSpawner, TaskSurfacePropagator,
};
pub use capability_policy::{
    builtin_registry_entry, is_non_delegating_session_source, resolve_document_policy,
    session_injected_capabilities, source_delegation_override_for_session_source,
    AgentRuntimePolicyResolver, HostSessionPolicyContext,
};
pub use capability_types::{
    normalize_runtime_name, CapabilityDisplayGroup, CapabilityKind, CapabilityRegistryEntry,
    ConfiguredBuiltinCapability, DocumentScopeMode, DocumentWriteMode, ResolvedDocumentPolicy,
    RuntimeCapabilitySnapshot, RuntimeDelivery, RuntimeExtensionResolution,
    RuntimeSkillResolution,
};
pub use prompt_composer::{
    build_base_business_prompt, build_prompt_introspection_snapshot, compose_top_level_prompt,
    compose_top_level_prompt_with_version, prompt_backup_manifest, render_prompt_snapshot,
    resolve_harness_capabilities, AgentPromptComposerInput, AgentPromptComposition,
    HarnessDelegationOverlay, PromptBackupManifest, PromptBackupSource, PromptCompositionReport,
    PromptIntrospectionSnapshot, PromptPackVersion, RenderedPromptSnapshot, SurfacePromptContract,
};
pub use provider_factory::{
    create_provider_for_config, HostApiFormat, HostProviderConfig, HostRuntimeOptimizationMode,
};
pub use tool_content::{ToolContentBlock, ToolTaskProgress, ToolTaskProgressCallback};
pub use tool_dispatch::execute_standard_tool_call;
pub use traits::{ToolDispatchExtensionState, ToolDispatchProgressSink};
