//! Invite service for team invitation management

use crate::error::TeamError;
use crate::models::{
    AcceptInviteResponse, CreateInviteRequest, CreateInviteResponse, TeamInvite,
    ValidateInviteResponse,
};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Service for managing team invites
pub struct InviteService;

impl InviteService {
    /// Create a new invite for a team
    pub async fn create_invite(
        pool: &SqlitePool,
        team_id: &str,
        creator_id: &str,
        request: CreateInviteRequest,
        base_url: &str,
    ) -> Result<CreateInviteResponse, TeamError> {
        // Generate a unique invite code (using short UUID)
        let code = Self::generate_invite_code();

        // Calculate expiration
        let expires_at = request.expires_in.expires_at();

        // Insert invite
        sqlx::query(
            r#"
            INSERT INTO team_invites (id, team_id, role, expires_at, max_uses, used_count, created_by, created_at, deleted)
            VALUES (?, ?, ?, ?, ?, 0, ?, ?, 0)
            "#,
        )
        .bind(&code)
        .bind(team_id)
        .bind(request.role.to_string())
        .bind(expires_at)
        .bind(request.max_uses)
        .bind(creator_id)
        .bind(Utc::now())
        .execute(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(CreateInviteResponse {
            code: code.clone(),
            url: format!("{}/join/{}", base_url.trim_end_matches('/'), code),
            expires_at,
            max_uses: request.max_uses,
            used_count: 0,
        })
    }

    /// Validate an invite code
    pub async fn validate_invite(
        pool: &SqlitePool,
        code: &str,
    ) -> Result<ValidateInviteResponse, TeamError> {
        // Get invite
        let invite: Option<TeamInvite> = sqlx::query_as(
            r#"
            SELECT 
                id, team_id, role, expires_at, max_uses, used_count, 
                created_by, created_at, deleted
            FROM team_invites
            WHERE id = ? AND deleted = 0
            "#,
        )
        .bind(code)
        .fetch_optional(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        let invite = match invite {
            Some(inv) => inv,
            None => {
                return Ok(ValidateInviteResponse {
                    valid: false,
                    team_id: None,
                    team_name: None,
                    team_description: None,
                    role: None,
                    inviter_name: None,
                    expires_at: None,
                    error: Some("Invite not found".to_string()),
                });
            }
        };

        // Get team info
        let team_info: Option<(String, Option<String>)> = sqlx::query_as(
            r#"
            SELECT name, description
            FROM teams
            WHERE id = ? AND is_deleted = 0
            "#,
        )
        .bind(&invite.team_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        let (team_name, team_description) =
            team_info.unwrap_or_else(|| ("Unknown".to_string(), None));

        // Get inviter name
        let inviter_name: Option<String> = sqlx::query_scalar(
            r#"
            SELECT display_name
            FROM team_members
            WHERE user_id = ? AND team_id = ? AND deleted = 0
            "#,
        )
        .bind(&invite.created_by)
        .bind(&invite.team_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?
        .flatten();

        if !invite.is_valid() {
            let error = if invite.expires_at.map(|e| Utc::now() > e).unwrap_or(false) {
                "Invite has expired"
            } else if invite
                .max_uses
                .map(|m| invite.used_count >= m)
                .unwrap_or(false)
            {
                "Invite has reached maximum uses"
            } else {
                "Invite is no longer valid"
            };

            return Ok(ValidateInviteResponse {
                valid: false,
                team_id: None,
                team_name: None,
                team_description: None,
                role: None,
                inviter_name: None,
                expires_at: None,
                error: Some(error.to_string()),
            });
        }

        Ok(ValidateInviteResponse {
            valid: true,
            team_id: Some(invite.team_id),
            team_name: Some(team_name),
            team_description,
            role: Some(invite.role),
            inviter_name,
            expires_at: invite.expires_at,
            error: None,
        })
    }

    /// Accept an invite and join the team
    pub async fn accept_invite(
        pool: &SqlitePool,
        code: &str,
        user_id: &str,
        display_name: &str,
    ) -> Result<AcceptInviteResponse, TeamError> {
        // Get and validate invite
        let invite: Option<TeamInvite> = sqlx::query_as(
            r#"
            SELECT id, team_id, role, expires_at, max_uses, used_count, created_by, created_at, deleted
            FROM team_invites
            WHERE id = ? AND deleted = 0
            "#,
        )
        .bind(code)
        .fetch_optional(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        let invite = match invite {
            Some(i) => i,
            None => {
                return Ok(AcceptInviteResponse {
                    success: false,
                    team_id: None,
                    member_id: None,
                    error: Some("Invite not found".to_string()),
                });
            }
        };

        if !invite.is_valid() {
            return Ok(AcceptInviteResponse {
                success: false,
                team_id: None,
                member_id: None,
                error: Some("Invite is no longer valid".to_string()),
            });
        }

        // Check if user is already a member
        let existing: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM team_members WHERE team_id = ? AND user_id = ? AND deleted = 0",
        )
        .bind(&invite.team_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        if existing.is_some() {
            return Ok(AcceptInviteResponse {
                success: false,
                team_id: Some(invite.team_id),
                member_id: None,
                error: Some("You are already a member of this team".to_string()),
            });
        }

        // Create member
        let member_id = Uuid::new_v4().to_string();
        let role = invite.get_role();

        sqlx::query(
            r#"
            INSERT INTO team_members (id, team_id, user_id, display_name, role, status, joined_at, deleted)
            VALUES (?, ?, ?, ?, ?, 'active', ?, 0)
            "#,
        )
        .bind(&member_id)
        .bind(&invite.team_id)
        .bind(user_id)
        .bind(display_name)
        .bind(role.to_string())
        .bind(Utc::now())
        .execute(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        // Increment invite used count
        sqlx::query("UPDATE team_invites SET used_count = used_count + 1 WHERE id = ?")
            .bind(code)
            .execute(pool)
            .await
            .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(AcceptInviteResponse {
            success: true,
            team_id: Some(invite.team_id),
            member_id: Some(member_id),
            error: None,
        })
    }

    /// List all invites for a team
    pub async fn list_invites(
        pool: &SqlitePool,
        team_id: &str,
    ) -> Result<Vec<TeamInvite>, TeamError> {
        let invites: Vec<TeamInvite> = sqlx::query_as(
            r#"
            SELECT id, team_id, role, expires_at, max_uses, used_count, created_by, created_at, deleted
            FROM team_invites
            WHERE team_id = ? AND deleted = 0
            ORDER BY created_at DESC
            "#,
        )
        .bind(team_id)
        .fetch_all(pool)
        .await
        .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(invites)
    }

    /// Delete (revoke) an invite
    pub async fn delete_invite(
        pool: &SqlitePool,
        code: &str,
        team_id: &str,
    ) -> Result<bool, TeamError> {
        let result =
            sqlx::query("UPDATE team_invites SET deleted = 1 WHERE id = ? AND team_id = ?")
                .bind(code)
                .bind(team_id)
                .execute(pool)
                .await
                .map_err(|e| TeamError::Database(e.to_string()))?;

        Ok(result.rows_affected() > 0)
    }

    /// Generate a short, URL-safe invite code
    fn generate_invite_code() -> String {
        // Generate a random code using base62 encoding
        use rand::Rng;
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut rng = rand::rng();
        (0..12)
            .map(|_| {
                let idx = rng.random_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }
}
