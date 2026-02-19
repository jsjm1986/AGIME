# Team Permission Model: Local vs Cloud Alignment

Date: 2026-02-12

## Design Position

- Local team mode is the execution environment.
- Cloud team mode is storage and sharing infrastructure.
- They should not be forced into identical behavior.
- Alignment should focus on permission metadata compatibility and install/authorization lifecycle, not on full feature parity.

## Local Permission Capabilities (Confirmed)

- Role and member permission checks exist (`owner/admin/member`, `can_share`, `can_install`, etc.).
- Resource protection levels exist (`public`, `team_installable`, `team_online_only`, `controlled`).
- Install rules enforce local-install eligibility by protection level.
- Team membership cleanup and expired-authorization cleanup logic exist.
- Runtime checks team skill authorization from `.skill-meta.json`.

## Key Inconsistencies Found (Before Patch)

- Skill API did not pass `protection_level` on create/update.
- Skill service did not persist `protection_level` in share/update/import insert paths.
- Remote-to-local `skills/install-local` wrote metadata without authorization payload.
- Remote UI install path did not fetch/send authorization token for non-public skills.
- Recipe/extension update versioning paths did not persist updated `protection_level`.

## Applied Patch

### 1) Skills permission field is now end-to-end

- Added `protection_level` to skills route API payloads for create/update.
- Added `protection_level` handling and persistence in skill service insert paths:
  - share insert
  - update(versioned) insert
  - import/create insert

Files:
- `crates/agime-team/src/routes/skills.rs`
- `crates/agime-team/src/services/skill_service.rs`

### 2) Local skill install now carries authorization metadata

- `POST /api/team/skills/install-local` now accepts optional `authorization` payload.
- For non-public protection levels:
  - uses client-provided authorization when present
  - keeps backward compatibility with a server-generated fallback token
- Metadata now writes `authorization` consistently.
- Added `installed_resources` upsert in local install path so cleanup/authorization management can track remote-installed local skills.

File:
- `crates/agime-team/src/routes/skills.rs`

### 3) Remote UI install now verifies access before local install

- Remote install flow now calls `verify-access` for skills requiring authorization.
- Passes `authorization { token, expiresAt, lastVerifiedAt }` into local install API.

File:
- `ui/desktop/src/components/team/api.ts`

### 4) Recipe/Extension update parity

- Versioned update insert now persists `protection_level` changes.

Files:
- `crates/agime-team/src/services/recipe_service.rs`
- `crates/agime-team/src/services/extension_service.rs`

### 5) Remote local-install tracking for Recipes/Extensions

- `POST /api/team/recipes/install-local` now writes `installed_resources` records.
- `POST /api/team/extensions/install-local` now writes `installed_resources` records.
- Both now:
  - enforce `protection_level` local-install policy
  - persist auth fields when present (with fallback token for non-public)
  - include `protection_level` in installation tracking

Files:
- `crates/agime-team/src/routes/recipes.rs`
- `crates/agime-team/src/routes/extensions.rs`

### 6) File-path cleanup compatibility

- Uninstall/cleanup now supports both directory paths and single-file paths.
- This is required because recipe local-install uses `recipes/{name}.yaml` file mode.

Files:
- `crates/agime-team/src/services/install_service.rs`
- `crates/agime-team/src/services/cleanup_service.rs`

### 7) Metadata compatibility + regression coverage for skill authorization

- Local runtime metadata parsing now accepts both `snake_case` and `camelCase` fields for team skill source/auth metadata.
- Added regression test for:
  - remote-installed non-public team skill
  - metadata with camelCase authorization fields
  - `loadSkill` authorization verification succeeds

Files:
- `crates/agime/src/agents/skills_extension.rs`
- `crates/agime/src/agents/team_extension.rs`

## Alignment Recommendation

Keep architecture split:

- Cloud: identity, storage, sharing, membership authority.
- Local: execution, cache/install, runtime enforcement, cleanup.

Align only these contracts:

- Shared enum values for `protection_level`.
- Authorization payload shape and lifetime fields.
- Install metadata schema used by runtime verification.
- Cleanup tracking contract in `installed_resources`.

Do not force cloud to mirror local package richness immediately; instead keep cloud schema backward-compatible and let local remain the richer execution side.
