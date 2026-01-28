//! Database migrations for team module

/// Migration SQL for team tables
pub const MIGRATION_SQL: &str = r#"
-- teams: 团队表
CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    repository_url TEXT,
    owner_id TEXT NOT NULL,
    is_deleted INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    settings_json TEXT DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_teams_owner ON teams(owner_id);
CREATE INDEX IF NOT EXISTS idx_teams_deleted ON teams(is_deleted);

-- team_members: 成员表
CREATE TABLE IF NOT EXISTS team_members (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    endpoint_url TEXT,
    role TEXT NOT NULL DEFAULT 'member',
    status TEXT NOT NULL DEFAULT 'active',
    permissions_json TEXT DEFAULT '{}',
    joined_at TEXT DEFAULT (datetime('now')),
    UNIQUE(team_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_members_team ON team_members(team_id);
CREATE INDEX IF NOT EXISTS idx_members_user ON team_members(user_id);

-- shared_skills: 共享 Skills
CREATE TABLE IF NOT EXISTS shared_skills (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    content TEXT NOT NULL,
    author_id TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    previous_version_id TEXT,
    visibility TEXT NOT NULL DEFAULT 'team',
    tags_json TEXT DEFAULT '[]',
    dependencies_json TEXT DEFAULT '[]',
    use_count INTEGER DEFAULT 0,
    is_deleted INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    UNIQUE(team_id, name, version)
);
CREATE INDEX IF NOT EXISTS idx_skills_team ON shared_skills(team_id);
CREATE INDEX IF NOT EXISTS idx_skills_author ON shared_skills(author_id);
CREATE INDEX IF NOT EXISTS idx_skills_name ON shared_skills(name);
CREATE INDEX IF NOT EXISTS idx_skills_deleted ON shared_skills(is_deleted);

-- shared_recipes: 共享 Recipes
CREATE TABLE IF NOT EXISTS shared_recipes (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    content_yaml TEXT NOT NULL,
    author_id TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    previous_version_id TEXT,
    visibility TEXT NOT NULL DEFAULT 'team',
    category TEXT,
    tags_json TEXT DEFAULT '[]',
    dependencies_json TEXT DEFAULT '[]',
    use_count INTEGER DEFAULT 0,
    is_deleted INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    UNIQUE(team_id, name, version)
);
CREATE INDEX IF NOT EXISTS idx_recipes_team ON shared_recipes(team_id);
CREATE INDEX IF NOT EXISTS idx_recipes_author ON shared_recipes(author_id);
CREATE INDEX IF NOT EXISTS idx_recipes_category ON shared_recipes(category);
CREATE INDEX IF NOT EXISTS idx_recipes_deleted ON shared_recipes(is_deleted);

-- shared_extensions: 共享 Extensions
CREATE TABLE IF NOT EXISTS shared_extensions (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    extension_type TEXT NOT NULL,
    config_json TEXT NOT NULL,
    author_id TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '1.0.0',
    previous_version_id TEXT,
    visibility TEXT NOT NULL DEFAULT 'team',
    tags_json TEXT DEFAULT '[]',
    security_reviewed INTEGER DEFAULT 0,
    security_notes TEXT,
    reviewed_by TEXT,
    reviewed_at TEXT,
    use_count INTEGER DEFAULT 0,
    is_deleted INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now')),
    UNIQUE(team_id, name, version)
);
CREATE INDEX IF NOT EXISTS idx_extensions_team ON shared_extensions(team_id);
CREATE INDEX IF NOT EXISTS idx_extensions_author ON shared_extensions(author_id);
CREATE INDEX IF NOT EXISTS idx_extensions_type ON shared_extensions(extension_type);
CREATE INDEX IF NOT EXISTS idx_extensions_deleted ON shared_extensions(is_deleted);

-- installed_resources: 已安装的团队资源
CREATE TABLE IF NOT EXISTS installed_resources (
    id TEXT PRIMARY KEY,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    team_id TEXT NOT NULL,
    resource_name TEXT NOT NULL,
    local_path TEXT,
    installed_version TEXT NOT NULL,
    latest_version TEXT,
    has_update INTEGER DEFAULT 0,
    installed_at TEXT DEFAULT (datetime('now')),
    last_checked_at TEXT,
    UNIQUE(resource_type, resource_id)
);
CREATE INDEX IF NOT EXISTS idx_installed_type ON installed_resources(resource_type);
CREATE INDEX IF NOT EXISTS idx_installed_team ON installed_resources(team_id);

-- sync_status: 同步状态
CREATE TABLE IF NOT EXISTS sync_status (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    last_sync_at TEXT,
    last_commit_hash TEXT,
    sync_state TEXT DEFAULT 'idle',
    error_message TEXT,
    UNIQUE(team_id)
);

-- resource_activities: 资源活动记录 (用于统计和推荐)
CREATE TABLE IF NOT EXISTS resource_activities (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    action TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_activities_user ON resource_activities(user_id);
CREATE INDEX IF NOT EXISTS idx_activities_resource ON resource_activities(resource_id);
CREATE INDEX IF NOT EXISTS idx_activities_type ON resource_activities(resource_type);
CREATE INDEX IF NOT EXISTS idx_activities_action ON resource_activities(action);
CREATE INDEX IF NOT EXISTS idx_activities_created ON resource_activities(created_at);

-- =====================================================
-- Composite indexes for optimized query performance
-- =====================================================

-- Skills: team + name lookup (without deleted filter in index, filter in query)
CREATE INDEX IF NOT EXISTS idx_skills_team_name ON shared_skills(team_id, name);
-- Skills: team + author lookup for "my skills in this team"
CREATE INDEX IF NOT EXISTS idx_skills_team_author ON shared_skills(team_id, author_id);
-- Skills: team + visibility for public/team visibility filtering
CREATE INDEX IF NOT EXISTS idx_skills_team_visibility ON shared_skills(team_id, visibility);
-- Skills: team + deleted for efficient active resource queries
CREATE INDEX IF NOT EXISTS idx_skills_team_deleted ON shared_skills(team_id, is_deleted);
-- Skills: use_count for sorting by popularity
CREATE INDEX IF NOT EXISTS idx_skills_use_count ON shared_skills(use_count DESC);

-- Recipes: composite indexes similar to skills
CREATE INDEX IF NOT EXISTS idx_recipes_team_name ON shared_recipes(team_id, name);
CREATE INDEX IF NOT EXISTS idx_recipes_team_author ON shared_recipes(team_id, author_id);
CREATE INDEX IF NOT EXISTS idx_recipes_team_visibility ON shared_recipes(team_id, visibility);
CREATE INDEX IF NOT EXISTS idx_recipes_team_deleted ON shared_recipes(team_id, is_deleted);
CREATE INDEX IF NOT EXISTS idx_recipes_team_category ON shared_recipes(team_id, category);
CREATE INDEX IF NOT EXISTS idx_recipes_use_count ON shared_recipes(use_count DESC);

-- Extensions: composite indexes similar to skills
CREATE INDEX IF NOT EXISTS idx_extensions_team_name ON shared_extensions(team_id, name);
CREATE INDEX IF NOT EXISTS idx_extensions_team_author ON shared_extensions(team_id, author_id);
CREATE INDEX IF NOT EXISTS idx_extensions_team_visibility ON shared_extensions(team_id, visibility);
CREATE INDEX IF NOT EXISTS idx_extensions_team_deleted ON shared_extensions(team_id, is_deleted);
CREATE INDEX IF NOT EXISTS idx_extensions_team_type ON shared_extensions(team_id, extension_type);
CREATE INDEX IF NOT EXISTS idx_extensions_use_count ON shared_extensions(use_count DESC);

-- Installed resources: type + team for efficient lookup
CREATE INDEX IF NOT EXISTS idx_installed_type_team ON installed_resources(resource_type, team_id);
-- Installed resources: has_update for finding resources with updates
CREATE INDEX IF NOT EXISTS idx_installed_has_update ON installed_resources(has_update) WHERE has_update = 1;

-- Activities: composite indexes for common queries
CREATE INDEX IF NOT EXISTS idx_activities_user_type ON resource_activities(user_id, resource_type);
CREATE INDEX IF NOT EXISTS idx_activities_resource_action ON resource_activities(resource_id, action);
CREATE INDEX IF NOT EXISTS idx_activities_user_created ON resource_activities(user_id, created_at DESC);

-- Team members: status filtering
CREATE INDEX IF NOT EXISTS idx_members_team_status ON team_members(team_id, status);
CREATE INDEX IF NOT EXISTS idx_members_team_role ON team_members(team_id, role);

-- =====================================================
-- Audit logs for compliance and security tracking
-- =====================================================

CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    timestamp TEXT NOT NULL DEFAULT (datetime('now')),
    user_id TEXT NOT NULL,
    action TEXT NOT NULL,
    resource_type TEXT,
    resource_id TEXT,
    team_id TEXT,
    details_json TEXT DEFAULT '{}',
    old_value_json TEXT,
    new_value_json TEXT,
    ip_address TEXT,
    user_agent TEXT,
    success INTEGER DEFAULT 1,
    error_message TEXT
);

-- Indexes for efficient audit log queries
CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_logs(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_audit_user ON audit_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_logs(action);
CREATE INDEX IF NOT EXISTS idx_audit_resource ON audit_logs(resource_type, resource_id);
CREATE INDEX IF NOT EXISTS idx_audit_team ON audit_logs(team_id);
CREATE INDEX IF NOT EXISTS idx_audit_success ON audit_logs(success);
CREATE INDEX IF NOT EXISTS idx_audit_user_timestamp ON audit_logs(user_id, timestamp DESC);

-- =====================================================
-- Add etag columns for optimistic locking
-- =====================================================

-- Note: ALTER TABLE ADD COLUMN with DEFAULT is safe in SQLite
-- These columns are used for optimistic concurrency control

-- Add etag to teams if not exists (SQLite will ignore if column exists due to IF NOT EXISTS on table)
-- We use a trigger-based approach since ALTER TABLE ADD COLUMN IF NOT EXISTS isn't supported
-- Instead, we create a new migration that's idempotent

-- For shared_skills: add update_counter for optimistic locking
-- This is handled via UPDATE ... WHERE update_counter = expected pattern

-- =====================================================
-- Resource locks for cooperative locking (optional)
-- =====================================================

CREATE TABLE IF NOT EXISTS resource_locks (
    id TEXT PRIMARY KEY,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    acquired_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT NOT NULL,
    UNIQUE(resource_type, resource_id)
);

CREATE INDEX IF NOT EXISTS idx_locks_resource ON resource_locks(resource_type, resource_id);
CREATE INDEX IF NOT EXISTS idx_locks_expires ON resource_locks(expires_at);

-- =====================================================
-- Fix member permissions (migration v9)
-- Sets default permissions to true for all existing members
-- =====================================================

UPDATE team_members
SET permissions_json = '{"can_share":true,"can_install":true,"can_delete_own":true}'
WHERE permissions_json = '{}'
   OR permissions_json = '{"can_share":false,"can_install":false,"can_delete_own":false}'
   OR permissions_json IS NULL;

-- =====================================================
-- Agent Skills Package Format Support (migration v10)
-- Adds package mode fields to shared_skills table
-- See: https://agentskills.io/specification
-- =====================================================

-- storage_type: 'inline' (default, backward compatible) or 'package'
-- SQLite doesn't support ADD COLUMN IF NOT EXISTS, so we use a workaround
-- by catching errors silently in the application layer

-- Add new columns for package mode support
-- Note: These ALTER statements will fail silently if columns already exist
-- The application handles this gracefully

-- Storage type: inline (simple text) or package (full package with files)
ALTER TABLE shared_skills ADD COLUMN storage_type TEXT DEFAULT 'inline';

-- SKILL.md content for package mode
ALTER TABLE shared_skills ADD COLUMN skill_md TEXT;

-- JSON array of files in the package
-- Format: [{"path": "scripts/lint.py", "content": "...", "content_type": "text/x-python", "size": 1234, "is_binary": false}]
ALTER TABLE shared_skills ADD COLUMN files_json TEXT;

-- Package manifest (JSON)
-- Format: {"scripts": ["scripts/lint.py"], "references": ["docs/guide.md"], "assets": ["templates/template.html"]}
ALTER TABLE shared_skills ADD COLUMN manifest_json TEXT;

-- Extended metadata from SKILL.md frontmatter (JSON)
-- Format: {"author": "...", "license": "MIT", "homepage": "...", "repository": "...", "keywords": [...], "estimated_tokens": 1000}
ALTER TABLE shared_skills ADD COLUMN metadata_json TEXT;

-- Package download URL (for large packages stored externally)
ALTER TABLE shared_skills ADD COLUMN package_url TEXT;

-- SHA-256 hash for package integrity verification
ALTER TABLE shared_skills ADD COLUMN package_hash TEXT;

-- Package size in bytes
ALTER TABLE shared_skills ADD COLUMN package_size INTEGER;

-- Index for filtering by storage type
CREATE INDEX IF NOT EXISTS idx_skills_storage_type ON shared_skills(storage_type);

-- Composite index for team + storage type queries
CREATE INDEX IF NOT EXISTS idx_skills_team_storage ON shared_skills(team_id, storage_type);

-- =====================================================
-- Protection Level Support (migration v11)
-- Implements tiered protection for team resources
-- Levels: public, team_installable (default), team_online_only, controlled
-- =====================================================

-- Protection level for skills
-- public: freely installable and copyable
-- team_installable: can be installed locally, requires authorization
-- team_online_only: cannot be installed locally, online access only
-- controlled: online only with full audit logging
ALTER TABLE shared_skills ADD COLUMN protection_level TEXT DEFAULT 'team_installable';

-- Protection level for recipes
ALTER TABLE shared_recipes ADD COLUMN protection_level TEXT DEFAULT 'team_installable';

-- Protection level for extensions
ALTER TABLE shared_extensions ADD COLUMN protection_level TEXT DEFAULT 'team_installable';

-- Indexes for protection level filtering
CREATE INDEX IF NOT EXISTS idx_skills_protection ON shared_skills(protection_level);
CREATE INDEX IF NOT EXISTS idx_recipes_protection ON shared_recipes(protection_level);
CREATE INDEX IF NOT EXISTS idx_extensions_protection ON shared_extensions(protection_level);

-- Composite indexes for team + protection level
CREATE INDEX IF NOT EXISTS idx_skills_team_protection ON shared_skills(team_id, protection_level);
CREATE INDEX IF NOT EXISTS idx_recipes_team_protection ON shared_recipes(team_id, protection_level);
CREATE INDEX IF NOT EXISTS idx_extensions_team_protection ON shared_extensions(team_id, protection_level);

-- =====================================================
-- Authorization tokens for installed resources (migration v11)
-- Stores authorization info for team resources installed locally
-- =====================================================

-- Add authorization columns to installed_resources
ALTER TABLE installed_resources ADD COLUMN user_id TEXT;
ALTER TABLE installed_resources ADD COLUMN authorization_token TEXT;
ALTER TABLE installed_resources ADD COLUMN authorization_expires_at TEXT;
ALTER TABLE installed_resources ADD COLUMN last_verified_at TEXT;
ALTER TABLE installed_resources ADD COLUMN protection_level TEXT DEFAULT 'team_installable';

-- Index for finding expired authorizations
CREATE INDEX IF NOT EXISTS idx_installed_auth_expires ON installed_resources(authorization_expires_at);
CREATE INDEX IF NOT EXISTS idx_installed_user ON installed_resources(user_id);
CREATE INDEX IF NOT EXISTS idx_installed_protection ON installed_resources(protection_level);

-- Composite index for cleanup queries (team + user)
CREATE INDEX IF NOT EXISTS idx_installed_team_user ON installed_resources(team_id, user_id);

-- =====================================================
-- Team Invites (migration v12)
-- Stores invitation links for team membership
-- =====================================================

CREATE TABLE IF NOT EXISTS team_invites (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member',
    expires_at TEXT,
    max_uses INTEGER,
    used_count INTEGER DEFAULT 0,
    created_by TEXT NOT NULL,
    created_at TEXT DEFAULT (datetime('now')),
    deleted INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_invites_team ON team_invites(team_id);
CREATE INDEX IF NOT EXISTS idx_invites_deleted ON team_invites(deleted);
CREATE INDEX IF NOT EXISTS idx_invites_expires ON team_invites(expires_at);
CREATE INDEX IF NOT EXISTS idx_invites_team_deleted ON team_invites(team_id, deleted);

-- Add deleted column to team_members if not exists
ALTER TABLE team_members ADD COLUMN deleted INTEGER DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_members_deleted ON team_members(deleted);

-- =====================================================
-- Unified Data Sources (migration v13)
-- Supports multi-source architecture with local caching
-- =====================================================

-- data_sources: 数据源配置表
CREATE TABLE IF NOT EXISTS data_sources (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,                    -- 'local' | 'cloud' | 'lan'
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    auth_type TEXT NOT NULL,               -- 'secret-key' | 'api-key'
    credential_encrypted TEXT,
    status TEXT DEFAULT 'offline',         -- 'online' | 'offline' | 'connecting' | 'error'
    teams_count INTEGER DEFAULT 0,
    last_sync_at TEXT,
    last_error TEXT,
    user_id TEXT,
    user_email TEXT,
    user_display_name TEXT,
    created_at TEXT DEFAULT (datetime('now')),
    updated_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_data_sources_type ON data_sources(type);
CREATE INDEX IF NOT EXISTS idx_data_sources_status ON data_sources(status);

-- cached_resources: 缓存的远程资源
CREATE TABLE IF NOT EXISTS cached_resources (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL,
    source_type TEXT NOT NULL,             -- 'cloud' | 'lan'
    resource_type TEXT NOT NULL,           -- 'skill' | 'recipe' | 'extension'
    resource_id TEXT NOT NULL,
    content_json TEXT NOT NULL,
    cached_at TEXT DEFAULT (datetime('now')),
    expires_at TEXT,
    sync_status TEXT DEFAULT 'synced',     -- 'synced' | 'local-only' | 'remote-only' | 'conflict' | 'pending'
    UNIQUE(source_id, resource_type, resource_id)
);

CREATE INDEX IF NOT EXISTS idx_cached_source ON cached_resources(source_id);
CREATE INDEX IF NOT EXISTS idx_cached_type ON cached_resources(resource_type);
CREATE INDEX IF NOT EXISTS idx_cached_expires ON cached_resources(expires_at);
CREATE INDEX IF NOT EXISTS idx_cached_sync_status ON cached_resources(sync_status);
CREATE INDEX IF NOT EXISTS idx_cached_source_type ON cached_resources(source_id, resource_type);

-- Add source_id to installed_resources for tracking which source a resource came from
ALTER TABLE installed_resources ADD COLUMN source_id TEXT DEFAULT 'local';
CREATE INDEX IF NOT EXISTS idx_installed_source ON installed_resources(source_id);

-- Insert default local data source
INSERT OR IGNORE INTO data_sources (id, type, name, url, auth_type, status)
VALUES ('local', 'local', 'Local', 'http://localhost:7778', 'secret-key', 'offline');
"#;

/// Run migration
pub async fn run_migration(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    // Split the SQL into individual statements and execute them
    for statement in MIGRATION_SQL.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() {
            // ALTER TABLE ADD COLUMN will fail if column already exists
            // This is expected behavior for idempotent migrations
            let result = sqlx::query(statement).execute(pool).await;

            // Ignore "duplicate column name" errors for ALTER TABLE statements
            if let Err(ref e) = result {
                let is_alter_table = statement.to_uppercase().contains("ALTER TABLE");
                let is_duplicate_column = e.to_string().contains("duplicate column name");

                if !(is_alter_table && is_duplicate_column) {
                    // Re-raise non-expected errors
                    result?;
                }
                // Otherwise, silently ignore (column already exists)
            }
        }
    }
    Ok(())
}
