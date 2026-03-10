# Digital Avatar Deep-Water Audit

## Scope

This audit covers the shared code paths between the digital-avatar channel and the ecosystem portal channel, with focus on the three highest-risk areas:

- governance source-of-truth split
- manager/service/owner binding invariants
- read paths with hidden write side effects

The goal is to define how to execute deep-water changes without changing current business behavior first.

## Current High-Risk Facts

### 1. Governance state has two competing write surfaces

- `crates/agime-team-server/src/agent/service_mongo.rs:2206`
  `get_avatar_governance_state` reads from `avatar_governance_states` first. If no state exists, it reconstructs governance from `portals.settings` and upserts back into `avatar_governance_states`.

- `crates/agime-team-server/src/agent/service_mongo.rs:2326`
  `update_avatar_governance_state` writes to `avatar_governance_states`, then also writes:
  - `settings.digitalAvatarGovernance`
  - `settings.digitalAvatarGovernanceConfig`
  back into the portal document.

- `crates/agime-team-server/src/agent/prompt_profiles.rs:125`
  The manager prompt explicitly tells the system to mutate governance through `settings_patch` at `settings.digitalAvatarGovernanceConfig`.

- `crates/agime-team-server/src/agent/portal_tools.rs:375`
  The tool schema for `configure_portal_service_agent` still documents governance mutation via `settings_patch.digitalAvatarGovernanceConfig`.

- `crates/agime-team-server/src/agent/portal_tools.rs:2153`
  The tool path now logs a warning when `settings_patch` mutates `digitalAvatarGovernance*`, which confirms this path is known to be a drift risk.

- `crates/agime-team-server/web-admin/src/pages/DigitalAvatarPolicyCenterPage.tsx:145`
  The policy center updates governance through the dedicated governance API.

- `crates/agime-team-server/web-admin/src/pages/DigitalAvatarPolicyCenterPage.tsx:146`
  The policy center also updates the portal document directly, which means the page is deliberately double-writing to keep both stores aligned.

Risk conclusion:

- governance state and governance config are not backed by a single authority
- different entry points mutate different surfaces
- the current system depends on compatibility writes, not a stable truth model

### 2. Avatar manager/service binding is not hardened at the common service layer

- `crates/agime-team/src/services/portal_service_mongo.rs:565`
  Avatar portal create enforces only:
  - `coding_agent_id` required
  - `service_agent_id` required
  - `coding_agent_id != service_agent_id`

- `crates/agime-team/src/services/portal_service_mongo.rs:417`
  `validate_service_agent_binding` only applies strict domain/role checks when `portal_domain == Ecosystem`.

- `crates/agime-team/src/services/portal_service_mongo.rs:613`
  Avatar portal create still calls `validate_service_agent_binding`, but the function returns early for avatar portals.

- `crates/agime-team/src/services/portal_service_mongo.rs:771`
  Avatar portal update preserves the same pattern: avatar bindings are checked for presence/distinctness, but not for avatar-specific domain/role/owner invariants.

- `crates/agime-team/src/services/portal_service_mongo.rs:844`
  Update only applies `validate_service_agent_binding` through the same ecosystem-focused logic.

- `crates/agime-team-server/src/agent/portal_tools.rs:897`
  The digital-avatar tool path validates that an avatar `service_agent_id` is a dedicated `digital_avatar/service`.

- `crates/agime-team-server/src/agent/portal_tools.rs:921`
  The tool path also validates that `service_agent.owner_manager_agent_id == manager_agent_id`.

- `crates/agime-team-server/src/agent/portal_tools.rs:1221`
  `create_digital_avatar` uses the stricter avatar-only path.

- `crates/agime-team-server/web-admin/src/components/team/digital-avatar/CreateAvatarDialog.tsx:193`
  The normal avatar creation UI provisions a `digital_avatar/service` agent.

- `crates/agime-team-server/web-admin/src/components/team/digital-avatar/CreateAvatarDialog.tsx:195`
  That UI also sets `owner_manager_agent_id`.

Risk conclusion:

- the avatar-only entry points are relatively safe
- the common Portal REST service does not define avatar manager/service/owner as model-level invariants
- cross-entry drift and historical dirty bindings are possible

### 3. Ecosystem sharing of `digital_avatar/service` is already a correct business capability

- `crates/agime-team-server/web-admin/src/components/team/portal/serviceAgentBinding.ts:15`
  Frontend classification explicitly supports:
  - `clone_general`
  - `direct_ecosystem`
  - `shared_avatar`

- `crates/agime-team-server/web-admin/src/components/team/portal/CreatePortalDialog.tsx:89`
  Ecosystem create flow allows `shared_avatar`.

- `crates/agime-team-server/web-admin/src/components/team/portal/PortalDetailView.tsx:637`
  Ecosystem detail flow also allows `shared_avatar`.

- `crates/agime-team/src/services/portal_service_mongo.rs:445`
  Backend accepts `digital_avatar/service` for ecosystem portal service binding.

- `crates/agime-team/src/services/portal_service_mongo.rs:450`
  Backend rejects general agents as direct ecosystem services.

- `crates/agime-team/src/services/portal_service_mongo.rs:453`
  Backend rejects `digital_avatar/manager`.

Risk conclusion:

- this is not the broken part
- this is a confirmed business rule and should be preserved

### 4. Governance ownership for shared avatar service is already pointed back to the avatar channel

- `crates/agime-team-server/web-admin/src/components/team/portal/PortalDetailView.tsx:1246`
- `crates/agime-team-server/web-admin/src/components/team/portal/PortalDetailView.tsx:1253`
- `crates/agime-team-server/web-admin/src/components/team/portal/PortalDetailView.tsx:1292`
- `crates/agime-team-server/web-admin/src/components/team/portal/PortalDetailView.tsx:1302`
- `crates/agime-team-server/web-admin/src/components/team/portal/PortalDetailView.tsx:1325`

The ecosystem portal detail page already says:

- this is a shared digital-avatar service
- governance still belongs to the digital-avatar channel
- operators should go back to avatar workspace / avatar timeline

Risk conclusion:

- the UI business narrative is correct
- the remaining risk is data linkage quality, not product definition

### 5. Several GET paths are not pure reads

- `crates/agime-team-server/src/agent/service_mongo.rs:2206`
  `get_avatar_governance_state` may:
  - upsert `avatar_governance_states`
  - seed `avatar_governance_events`

- `crates/agime-team-server/src/agent/service_mongo.rs:2462`
  `list_avatar_governance_queue` calls `get_avatar_governance_state`, so queue reads inherit those write side effects.

- `crates/agime-team-server/src/agent/service_mongo.rs:2561`
  `list_avatar_instance_projections` calls `sync_avatar_instance_projections`.

- `crates/agime-team-server/src/agent/service_mongo.rs:2150`
  `sync_avatar_instance_projections` scans avatar portals and writes projection rows back to `avatar_instances`.

- `crates/agime-team-server/src/agent/service_mongo.rs:2889`
  `get_avatar_workbench_snapshot` reads workbench data but also rebuilds derived manager reports.

- `crates/agime-team-server/src/agent/service_mongo.rs:3036`
  `get_avatar_workbench_snapshot` calls `replace_avatar_manager_reports`, which writes to `avatar_manager_reports`.

- `crates/agime-team-server/src/agent/service_mongo.rs:1706`
  `seed_avatar_governance_events_if_missing` is a write helper that is invoked from read flows.

- `crates/agime-team-server/src/agent/service_mongo.rs:2471`
  `backfill_avatar_governance_storage` is an explicit repair path that reads portal data and writes governance state/events/projections.

Risk conclusion:

- viewing avatar state is not always read-only
- observability pages can mutate storage
- migration and hardening work should not proceed until these read/write boundaries are mapped

## Deep-Water Execution Strategy

### Phase 0: Already in Place

Completed before entering deep-water changes:

- regression matrix document
- backend guard-rail tests
- avatar frontend API wrapper
- shadow warnings for governance drift and orphan governance state
- shared frontend binding helpers for ecosystem service-agent classification

### Phase 1: Inventory and Data Audit

Do not change behavior yet. Produce data reports from real storage.

Audit report A: governance divergence

- list all avatar portals
- compare:
  - `avatar_governance_states.config`
  - `portals.settings.digitalAvatarGovernanceConfig`
  - `portals.settings.digitalAvatarGovernance`
- classify each row:
  - in sync
  - state-only
  - settings-only
  - both exist but differ

Audit report B: avatar binding anomalies

- list all avatar portals
- verify:
  - `coding_agent_id` is `digital_avatar/manager`
  - `service_agent_id` is `digital_avatar/service`
  - `coding_agent_id != service_agent_id`
  - `service.owner_manager_agent_id == coding_agent_id`
- classify each row:
  - valid
  - missing manager
  - missing service
  - wrong manager role
  - wrong service role
  - owner mismatch
  - same agent reused

Audit report C: read-path write map

- document every GET or read workflow that writes:
  - governance state
  - governance events
  - avatar projections
  - avatar manager reports

Exit criteria for Phase 1:

- all three reports exist
- dirty-state counts are known
- no runtime behavior has changed

### Phase 2: Shadow Mode

Still no hard blocking. Add comparison and warning only.

- governance writes compute what the canonical state would be, but continue current writes
- avatar portal create/update computes full invariant checks, but logs violations instead of rejecting
- read paths emit metrics/warnings when they trigger a write side effect

Exit criteria for Phase 2:

- we know how often drift would be hit
- we know whether hardening would break real traffic

### Phase 3: Backfill and Repair

Repair data before changing truth rules.

- fix avatar portal manager/service/owner mismatches
- repair or remove orphan governance state rows
- re-seed or normalize governance projections where needed
- ensure shared avatar service links can resolve back to avatar workspace records

Exit criteria for Phase 3:

- historical dirty rows are reduced to zero or to an approved exception list

### Phase 4: Write-Path Cutover

Cut new writes first, keep reads compatible.

Recommended order:

- governance writes go through a single governance service
- direct `settings_patch.digitalAvatarGovernance*` remains compatibility-only and warning-producing
- avatar create/update in common `PortalService` starts enforcing avatar invariants for new writes
- ecosystem shared-avatar behavior remains unchanged

Exit criteria for Phase 4:

- new dirty data can no longer be created through normal write paths

### Phase 5: Read-Path Cutover

Only after writes are stable.

- move GET flows toward pure reads
- move projection rebuild, governance seed, and manager-report derivation into explicit jobs or write-triggered sync
- keep explicit repair/backfill commands separate from page reads

Exit criteria for Phase 5:

- user reads no longer mutate governance or projection storage

### Phase 6: Cleanup

- remove deprecated compatibility reads/writes
- shrink duplicated governance fields
- keep only the approved long-term truth model

## Decisions Still Requiring Product Approval

### Decision A: governance final source of truth

Recommended:

- canonical truth: `avatar_governance_states`
- compatibility projection: `portals.settings.digitalAvatarGovernanceConfig`

Reason:

- events, queue, counts, and workbench are already organized around governance storage collections
- this reduces future drift caused by portal-level ad hoc patches

### Decision B: publish hard gate activation

Do not activate directly.

Recommended path:

- first compute would-block decisions in shadow mode
- then show would-block state in admin UI
- only after approval, enforce hard publish blocking

## Immediate Next Steps

1. Build the three data-audit reports against real storage.
2. Review dirty-row counts with product/engineering.
3. Lock the governance final source of truth.
4. Harden new writes before touching legacy reads.
