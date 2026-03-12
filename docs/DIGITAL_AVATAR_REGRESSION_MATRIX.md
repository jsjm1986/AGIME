# Digital Avatar Regression Matrix

## Purpose

This matrix locks the current business boundary between the digital-avatar channel and the ecosystem portal channel. Use it before and after any refactor in shared portal, agent-binding, governance, or web-admin code.

## Non-Negotiable Business Rules

- Ecosystem portals may directly bind a digital-avatar `service` agent as a shared service.
- Ecosystem portals may not bind a digital-avatar `manager` agent as their `service_agent_id`.
- General agents may not be bound directly as `service_agent_id` for ecosystem portals. They must be cloned into an ecosystem service agent first.
- Digital-avatar manual pages may keep using the common portal update path. Internal wrappers may exist, but external behavior must remain unchanged.
- Shared avatar service governance still belongs to the digital-avatar channel, not the ecosystem channel.

## Shared Backend Guard Rails

- Ecosystem service-agent binding validation: `crates/agime-team/src/services/portal_service_mongo.rs`
- Current backend binding matrix helper: `classify_ecosystem_service_agent_binding`
- Digital-avatar governance compatibility warnings: `crates/agime-team-server/src/agent/service_mongo.rs`
- Portal-tool governance drift warning: `crates/agime-team-server/src/agent/portal_tools.rs`

## Shared Frontend Guard Rails

- Ecosystem service-agent classification and wording helper: `crates/agime-team-server/web-admin/src/components/team/portal/serviceAgentBinding.ts`
- Digital-avatar portal wrapper over common portal API: `crates/agime-team-server/web-admin/src/api/avatarPortal.ts`

## Automated Verification

Run all of these after changing shared portal or digital-avatar code:

```powershell
cargo test -p agime-team services::mongo::portal_service_mongo::tests -- --nocapture
cargo test -p agime-team-server governance_ -- --nocapture
cd crates/agime-team-server/web-admin
npm run typecheck
npm run build
```

## Manual Regression Matrix

### Ecosystem Create Portal

- Create an ecosystem portal from a general agent template.
- Expected: the selected general agent is cloned into an `ecosystem_portal/service` agent.
- Verify the create dialog still groups service agents into general templates, ecosystem services, shared avatar services, and blocked entries.

- Create an ecosystem portal by selecting an existing `ecosystem_portal/service` agent.
- Expected: the portal binds the selected service agent directly.

- Create an ecosystem portal by selecting an existing `digital_avatar/service` agent.
- Expected: the portal binds the selected avatar service agent directly as a shared service.
- Expected: the UI explains that governance ownership remains in the digital-avatar channel.

- Try selecting a `digital_avatar/manager` agent.
- Expected: the UI blocks the action and shows the existing invalid-manager message.
- Expected: backend validation also rejects the binding if the UI path is bypassed.

### Ecosystem Portal Detail

- Open an ecosystem portal that uses a shared avatar service.
- Expected: the service-governance ownership panel is visible.
- Expected: the page shows links back to the digital-avatar workspace and timeline when the linked avatar can be resolved.
- Expected: if the linked avatar cannot be resolved, the warning still says governance belongs to the digital-avatar channel.

- Open an ecosystem portal that uses an ecosystem dedicated service.
- Expected: the page shows ecosystem ownership wording, not digital-avatar ownership wording.

- Open an ecosystem portal that was created from a general template.
- Expected: the page reflects dedicated ecosystem service behavior after save.

### Ecosystem Portal List

- Open the ecosystem portal list.
- Expected: cards still show the correct service-mode badge for:
- `shared_avatar`
- `direct_ecosystem`
- `clone_general`

### Digital Avatar Create and Manage

- Create a manager agent from a general agent template.
- Expected: only general agents appear as manager templates.

- Create a digital avatar from a manager agent.
- Expected: the avatar uses the manager as `codingAgentId`.
- Expected: the avatar creates or binds a dedicated service agent according to the current flow.

- Save avatar manual settings from the avatar manager page.
- Expected: document bindings, document access mode, allowed extensions, and allowed skills still persist through the common portal update path.

### Digital Avatar Workspace and Governance

- Open the digital-avatar workspace.
- Expected: avatar list, governance events, governance queue, and workbench snapshot still load.

- Update avatar governance from the workspace.
- Expected: state/config updates still persist and the workspace refreshes correctly.

- Update avatar governance from the timeline page.
- Expected: behavior matches the workspace path.

- Open the policy center and apply policy to existing avatars.
- Expected: governance config and portal settings remain aligned with current behavior.

- Open the audit center and overview page.
- Expected: avatar list, projections, governance queues, and team governance events still load.

### Publish Path

- Publish and unpublish a digital avatar from the workspace.
- Expected: current behavior remains unchanged.
- Expected: no new hard gate is introduced unless explicitly approved as a business change.

## Warning Signals To Inspect

- `avatar governance requested with invalid portal id`
- `avatar governance requested with invalid team id`
- `avatar governance requested for missing portal`
- `avatar governance requested for non-avatar portal`
- `avatar governance state updated without a matching portal document`
- `portal settings_patch is mutating digital avatar governance data`

## Stop Conditions

- Stop if ecosystem can no longer bind a shared avatar service directly.
- Stop if digital-avatar manual save stops working through the common portal update path.
- Stop if a shared avatar service no longer points governance users back to the digital-avatar channel.
- Stop if create/update behavior changes for manager/service agent pairing without explicit product approval.
