//! Member service - business logic for member operations

use chrono::Utc;
use sqlx::SqlitePool;
use std::path::PathBuf;
use tracing::{info, warn};

use crate::error::{TeamError, TeamResult};
use crate::models::{
    AddMemberRequest, ListMembersQuery, MemberPermissions, MemberRole, MemberStatus,
    PaginatedResponse, TeamMember,
};
use crate::services::{AuditAction, AuditService, CleanupResult};

/// Member service
pub struct MemberService;

impl MemberService {
    /// Create a new member service
    pub fn new() -> Self {
        Self
    }

    /// Add a member to a team
    pub async fn add_member(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        request: AddMemberRequest,
    ) -> TeamResult<TeamMember> {
        let role = request.role.unwrap_or(MemberRole::Member);

        let member = TeamMember {
            id: uuid::Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            user_id: request.user_id,
            display_name: request.display_name,
            endpoint_url: request.endpoint_url,
            role,
            status: MemberStatus::Active,
            permissions: MemberPermissions::default(),
            joined_at: Utc::now(),
        };

        let permissions_json = serde_json::to_string(&member.permissions)?;

        sqlx::query(
            r#"
            INSERT INTO team_members (id, team_id, user_id, display_name, endpoint_url, role, status, permissions_json, joined_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&member.id)
        .bind(&member.team_id)
        .bind(&member.user_id)
        .bind(&member.display_name)
        .bind(&member.endpoint_url)
        .bind(member.role.to_string())
        .bind(member.status.to_string())
        .bind(&permissions_json)
        .bind(member.joined_at.to_rfc3339())
        .execute(pool)
        .await?;

        Ok(member)
    }

    /// Get a member by ID
    pub async fn get_member(&self, pool: &SqlitePool, member_id: &str) -> TeamResult<TeamMember> {
        let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, String, String, String)>(
            r#"
            SELECT id, team_id, user_id, display_name, endpoint_url, role, status, permissions_json, joined_at
            FROM team_members
            WHERE id = ?
            "#,
        )
        .bind(member_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| TeamError::MemberNotFound(member_id.to_string()))?;

        let permissions: MemberPermissions = serde_json::from_str(&row.7).unwrap_or_default();

        Ok(TeamMember {
            id: row.0,
            team_id: row.1,
            user_id: row.2,
            display_name: row.3,
            endpoint_url: row.4,
            role: row.5.parse().unwrap_or(MemberRole::Member),
            status: row.6.parse().unwrap_or_default(),
            permissions,
            joined_at: chrono::DateTime::parse_from_rfc3339(&row.8)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    /// Get a member by user ID in a team
    pub async fn get_member_by_user(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        user_id: &str,
    ) -> TeamResult<TeamMember> {
        let row = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, String, String, String)>(
            r#"
            SELECT id, team_id, user_id, display_name, endpoint_url, role, status, permissions_json, joined_at
            FROM team_members
            WHERE team_id = ? AND user_id = ?
            "#,
        )
        .bind(team_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| TeamError::MemberNotFound(user_id.to_string()))?;

        let permissions: MemberPermissions = serde_json::from_str(&row.7).unwrap_or_default();

        Ok(TeamMember {
            id: row.0,
            team_id: row.1,
            user_id: row.2,
            display_name: row.3,
            endpoint_url: row.4,
            role: row.5.parse().unwrap_or(MemberRole::Member),
            status: row.6.parse().unwrap_or_default(),
            permissions,
            joined_at: chrono::DateTime::parse_from_rfc3339(&row.8)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    /// List members of a team
    pub async fn list_members(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        query: ListMembersQuery,
    ) -> TeamResult<PaginatedResponse<TeamMember>> {
        let offset = (query.page.saturating_sub(1)) * query.limit;

        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM team_members WHERE team_id = ?")
            .bind(team_id)
            .fetch_one(pool)
            .await?;

        let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, String, String, String)>(
            r#"
            SELECT id, team_id, user_id, display_name, endpoint_url, role, status, permissions_json, joined_at
            FROM team_members
            WHERE team_id = ?
            ORDER BY joined_at DESC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(team_id)
        .bind(query.limit as i64)
        .bind(offset as i64)
        .fetch_all(pool)
        .await?;

        let members: Vec<TeamMember> = rows
            .into_iter()
            .map(|row| {
                let permissions: MemberPermissions =
                    serde_json::from_str(&row.7).unwrap_or_default();
                TeamMember {
                    id: row.0,
                    team_id: row.1,
                    user_id: row.2,
                    display_name: row.3,
                    endpoint_url: row.4,
                    role: row.5.parse().unwrap_or(MemberRole::Member),
                    status: row.6.parse().unwrap_or_default(),
                    permissions,
                    joined_at: chrono::DateTime::parse_from_rfc3339(&row.8)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                }
            })
            .collect();

        Ok(PaginatedResponse::new(
            members,
            total.0 as u64,
            query.page,
            query.limit,
        ))
    }

    /// Remove a member from a team
    pub async fn remove_member(&self, pool: &SqlitePool, member_id: &str) -> TeamResult<()> {
        sqlx::query("DELETE FROM team_members WHERE id = ?")
            .bind(member_id)
            .execute(pool)
            .await?;

        Ok(())
    }

    /// Remove a member from a team with resource cleanup
    /// This is the preferred method when a member leaves or is removed
    /// DATA-1 FIX: Wrap database operations in transaction for atomicity
    pub async fn remove_member_with_cleanup(
        &self,
        pool: &SqlitePool,
        member_id: &str,
        _base_path: &PathBuf,
        requester_id: &str,
    ) -> TeamResult<RemoveMemberResult> {
        // 1. Get member info before deletion
        let member = self.get_member(pool, member_id).await?;

        info!(
            member_id = %member_id,
            team_id = %member.team_id,
            user_id = %member.user_id,
            "Removing member with resource cleanup"
        );

        // 2. Get list of resources to clean up (for file cleanup after transaction)
        let resources: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
            r#"
            SELECT id, resource_type, resource_name, local_path
            FROM installed_resources
            WHERE team_id = ? AND user_id = ?
            "#,
        )
        .bind(&member.team_id)
        .bind(&member.user_id)
        .fetch_all(pool)
        .await?;

        // 3. Start transaction for atomic database operations
        let mut tx = pool.begin().await?;

        // 4. Delete all installed resources for this user in this team
        let delete_result =
            sqlx::query("DELETE FROM installed_resources WHERE team_id = ? AND user_id = ?")
                .bind(&member.team_id)
                .bind(&member.user_id)
                .execute(&mut *tx)
                .await?;

        let db_deleted_count = delete_result.rows_affected() as usize;

        // 5. Delete member record
        sqlx::query("DELETE FROM team_members WHERE id = ?")
            .bind(member_id)
            .execute(&mut *tx)
            .await?;

        // 6. Commit transaction
        tx.commit().await?;

        info!(
            db_deleted = db_deleted_count,
            "Database cleanup completed in transaction"
        );

        // 7. Clean up local files (after transaction commit, so even if this fails, DB is consistent)
        let mut file_cleanup_failures = Vec::new();
        let mut bytes_freed = 0u64;

        for (id, resource_type, resource_name, local_path) in &resources {
            if let Some(path_str) = local_path {
                let path = PathBuf::from(path_str);
                if path.exists() {
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if metadata.is_dir() {
                            bytes_freed +=
                                crate::services::cleanup_service::calculate_dir_size(&path)
                                    .unwrap_or(0);
                        } else {
                            bytes_freed += metadata.len();
                        }
                    }

                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        warn!(
                            path = %path_str,
                            error = %e,
                            "Failed to delete local files (DB already cleaned)"
                        );
                        file_cleanup_failures.push(
                            crate::services::cleanup_service::CleanupFailure {
                                resource_type: resource_type
                                    .parse()
                                    .unwrap_or(crate::models::ResourceType::Skill),
                                resource_id: id.clone(),
                                resource_name: resource_name.clone(),
                                error: format!("Failed to delete files: {}", e),
                            },
                        );
                    }
                }
            }
        }

        let cleanup_result = crate::services::cleanup_service::CleanupResult {
            cleaned_count: db_deleted_count,
            failures: file_cleanup_failures,
            bytes_freed,
        };

        info!(
            cleaned = cleanup_result.cleaned_count,
            failures = cleanup_result.failures.len(),
            "Resource cleanup completed"
        );

        // 8. Record audit log (non-critical, don't fail if this fails)
        let audit_service = AuditService::new();
        let details_json = serde_json::to_string(&serde_json::json!({
            "removed_user_id": member.user_id,
            "removed_display_name": member.display_name,
            "removed_role": member.role.to_string(),
            "cleaned_resources": cleanup_result.cleaned_count,
            "cleanup_failures": cleanup_result.failures.len(),
        }))
        .ok();
        let _ = audit_service
            .log(
                pool,
                requester_id,
                AuditAction::MemberRemove,
                Some("member"),
                Some(member_id),
                Some(&member.team_id),
                details_json
                    .as_ref()
                    .and_then(|s| serde_json::from_str(s).ok()),
                None,
                None,
                None,
                None,
            )
            .await;

        Ok(RemoveMemberResult {
            member_id: member_id.to_string(),
            team_id: member.team_id,
            user_id: member.user_id,
            cleanup_result,
        })
    }

    /// Member leaves a team voluntarily with cleanup
    pub async fn leave_team_with_cleanup(
        &self,
        pool: &SqlitePool,
        team_id: &str,
        user_id: &str,
        base_path: &PathBuf,
    ) -> TeamResult<RemoveMemberResult> {
        // Get member info
        let member = self.get_member_by_user(pool, team_id, user_id).await?;

        // Owners cannot leave
        if member.is_owner() {
            return Err(TeamError::OwnerCannotLeave);
        }

        // Use the cleanup removal method
        self.remove_member_with_cleanup(pool, &member.id, base_path, user_id)
            .await
    }

    /// Update a member's role and/or display name
    pub async fn update_member(
        &self,
        pool: &SqlitePool,
        member_id: &str,
        role: Option<MemberRole>,
        display_name: Option<String>,
    ) -> TeamResult<TeamMember> {
        let mut member = self.get_member(pool, member_id).await?;

        // Update fields if provided
        if let Some(new_role) = role {
            member.role = new_role;
        }
        if let Some(new_name) = display_name {
            member.display_name = new_name;
        }

        // Save to database
        sqlx::query("UPDATE team_members SET role = ?, display_name = ? WHERE id = ?")
            .bind(member.role.to_string())
            .bind(&member.display_name)
            .bind(member_id)
            .execute(pool)
            .await?;

        Ok(member)
    }
}

impl Default for MemberService {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of removing a member with cleanup
#[derive(Debug, Clone)]
pub struct RemoveMemberResult {
    /// The removed member's ID
    pub member_id: String,
    /// The team ID the member was removed from
    pub team_id: String,
    /// The user ID of the removed member
    pub user_id: String,
    /// Result of resource cleanup
    pub cleanup_result: CleanupResult,
}

impl RemoveMemberResult {
    /// Check if removal was completely successful (including cleanup)
    pub fn is_complete(&self) -> bool {
        self.cleanup_result.is_complete()
    }
}
