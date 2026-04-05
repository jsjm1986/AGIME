//! Team service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{
    CreateTeamRequest, MemberPermissions, Team, TeamInvite, TeamMember, TeamSettings, TeamSummary,
};
use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use mongodb::options::FindOptions;
use serde::Deserialize;
use uuid::Uuid;

pub struct TeamService {
    db: MongoDb,
}

#[derive(Debug, Deserialize)]
struct TeamListRow {
    #[serde(rename = "_id", default)]
    id: Option<ObjectId>,
    name: String,
    description: Option<String>,
    repository_url: Option<String>,
    owner_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    created_at: chrono::DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
struct ActiveUserProfile {
    user_id: String,
    email: String,
    display_name: String,
}

#[derive(Debug, Clone)]
pub struct InviteResolution {
    pub team_id: String,
    pub team_name: String,
    pub role: String,
    pub invitee_email: Option<String>,
    pub is_open_invite: bool,
}

impl From<TeamListRow> for TeamSummary {
    fn from(team: TeamListRow) -> Self {
        Self {
            id: team.id.map(|id| id.to_hex()).unwrap_or_default(),
            name: team.name,
            description: team.description,
            repository_url: team.repository_url,
            owner_id: team.owner_id,
            created_at: team.created_at.to_rfc3339(),
            updated_at: team.updated_at.to_rfc3339(),
        }
    }
}

impl TeamService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    async fn load_active_user_profile(&self, user_id: &str) -> Result<ActiveUserProfile> {
        let coll = self.db.collection::<ActiveUserProfile>("users");
        coll.find_one(doc! { "user_id": user_id, "is_active": true }, None)
            .await?
            .ok_or_else(|| anyhow!("User not found"))
    }

    fn normalize_member_role(role: &str) -> Result<String> {
        let normalized = role.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "admin" | "member" => Ok(normalized),
            "owner" => Err(anyhow!(
                "Owner role cannot be assigned via member operations"
            )),
            _ => Err(anyhow!("Invalid member role")),
        }
    }

    fn normalize_invitee_email(email: &str) -> Result<String> {
        let normalized = email.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return Err(anyhow!("Invite email is required"));
        }
        let parts: Vec<&str> = normalized.split('@').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() || !parts[1].contains('.')
        {
            return Err(anyhow!("Invalid invite email"));
        }
        Ok(normalized)
    }

    fn ensure_invitee_email_matches(expected: &str, actual: &str) -> Result<()> {
        let normalized_actual = Self::normalize_invitee_email(actual)?;
        if expected != normalized_actual {
            return Err(anyhow!("This invite is only valid for {}", expected));
        }
        Ok(())
    }

    fn normalize_open_invite_days(expires_in_days: Option<i64>) -> Result<i64> {
        let days = expires_in_days
            .ok_or_else(|| anyhow!("Open invite links must set an expiry between 1 and 7 days"))?;
        if !(1..=7).contains(&days) {
            return Err(anyhow!("Open invite links must expire within 1 to 7 days"));
        }
        Ok(days)
    }

    async fn resolve_valid_invite(&self, code: &str) -> Result<(TeamInvite, Team)> {
        let invite = self
            .get_invite_by_code(code)
            .await?
            .ok_or_else(|| anyhow!("Invalid invite code"))?;

        if let Some(expires_at) = invite.expires_at {
            if expires_at < Utc::now() {
                return Err(anyhow!("Invite has expired"));
            }
        }

        if let Some(max) = invite.max_uses {
            if invite.used_count >= max {
                return Err(anyhow!("Invite has reached maximum uses"));
            }
        }

        Self::normalize_member_role(&invite.role)?;

        if invite.is_open_invite {
            if invite.expires_at.is_none() {
                return Err(anyhow!(
                    "Invalid open invite configuration. Generate a new invite link."
                ));
            }
        } else {
            Self::normalize_invitee_email(&invite.invitee_email).map_err(|_| {
                anyhow!("Legacy invite links are no longer supported. Generate a new invite link.")
            })?;
        }

        let team_id = invite.team_id.to_hex();
        let team = self
            .get(&team_id)
            .await?
            .ok_or_else(|| anyhow!("Team no longer exists"))?;

        Ok((invite, team))
    }

    pub async fn get_valid_invite_details(&self, code: &str) -> Result<(TeamInvite, Team)> {
        self.resolve_valid_invite(code).await
    }

    pub async fn validate_invite_for_registration(
        &self,
        code: &str,
        email: &str,
    ) -> Result<InviteResolution> {
        let (invite, team) = self.resolve_valid_invite(code).await?;
        if !invite.is_open_invite {
            Self::ensure_invitee_email_matches(&invite.invitee_email, email)?;
        }
        Ok(InviteResolution {
            team_id: invite.team_id.to_hex(),
            team_name: team.name,
            role: invite.role,
            invitee_email: if invite.is_open_invite {
                None
            } else {
                Some(invite.invitee_email)
            },
            is_open_invite: invite.is_open_invite,
        })
    }

    pub async fn sync_member_profile(
        &self,
        user_id: &str,
        email: &str,
        display_name: &str,
    ) -> Result<u64> {
        let coll = self.db.collection::<Team>("teams");
        let mut cursor = coll
            .find(
                doc! {
                    "is_deleted": { "$ne": true },
                    "members.user_id": user_id,
                },
                None,
            )
            .await?;
        let mut updated_teams = 0_u64;
        let now = bson::DateTime::from_chrono(Utc::now());

        while let Some(team) = cursor.try_next().await? {
            let Some(team_id) = team.id.clone() else {
                continue;
            };

            let mut changed = false;
            let mut next_members = team.members;

            for member in &mut next_members {
                if member.user_id == user_id {
                    if member.email != email {
                        member.email = email.to_string();
                        changed = true;
                    }
                    if member.display_name != display_name {
                        member.display_name = display_name.to_string();
                        changed = true;
                    }
                }
            }

            if !changed {
                continue;
            }

            coll.update_one(
                doc! { "_id": team_id },
                doc! {
                    "$set": {
                        "members": bson::to_bson(&next_members)?,
                        "updated_at": now,
                    }
                },
                None,
            )
            .await?;
            updated_teams += 1;
        }

        Ok(updated_teams)
    }

    /// Create a new team
    pub async fn create(&self, user_id: &str, req: CreateTeamRequest) -> Result<Team> {
        let now = Utc::now();
        let owner_profile = self.load_active_user_profile(user_id).await?;
        let owner_display_name = if owner_profile.display_name.trim().is_empty() {
            owner_profile.email.clone()
        } else {
            owner_profile.display_name.clone()
        };
        let team = Team {
            id: None,
            name: req.name,
            description: req.description,
            repository_url: None,
            owner_id: user_id.to_string(),
            members: vec![TeamMember {
                user_id: owner_profile.user_id,
                email: owner_profile.email,
                display_name: owner_display_name,
                role: "owner".to_string(),
                status: "active".to_string(),
                permissions: MemberPermissions::default(),
                joined_at: now,
            }],
            settings: TeamSettings::default(),
            is_deleted: false,
            created_at: now,
            updated_at: now,
        };

        let coll = self.db.collection::<Team>("teams");
        let result = coll.insert_one(&team, None).await?;

        let mut team = team;
        team.id = result.inserted_id.as_object_id();
        Ok(team)
    }

    /// List teams for a user
    pub async fn list_for_user(&self, user_id: &str) -> Result<Vec<TeamSummary>> {
        let coll = self.db.collection::<TeamListRow>("teams");
        // Filter out soft-deleted teams
        let filter = doc! {
            "is_deleted": { "$ne": true },
            "$or": [
                { "owner_id": user_id },
                { "members.user_id": user_id }
            ]
        };
        let options = FindOptions::builder()
            .projection(doc! {
                "_id": 1,
                "name": 1,
                "description": 1,
                "repository_url": 1,
                "owner_id": 1,
                "created_at": 1,
                "updated_at": 1,
            })
            .build();

        let cursor = coll.find(filter, options).await?;
        let teams: Vec<TeamListRow> = cursor.try_collect().await?;

        Ok(teams.into_iter().map(TeamSummary::from).collect())
    }

    /// Get team by ID
    pub async fn get(&self, team_id: &str) -> Result<Option<Team>> {
        let oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Team>("teams");
        // Filter out soft-deleted teams
        Ok(coll
            .find_one(doc! { "_id": oid, "is_deleted": { "$ne": true } }, None)
            .await?)
    }

    /// Delete team (soft delete) with cascade
    pub async fn delete(&self, team_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(team_id)?;
        let now = bson::DateTime::from_chrono(Utc::now());

        // Soft delete the team
        let coll = self.db.collection::<Team>("teams");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$set": { "is_deleted": true, "updated_at": now } },
            None,
        )
        .await?;

        // Cascade soft delete all related resources
        let update = doc! { "$set": { "is_deleted": true, "updated_at": now } };
        let filter = doc! { "team_id": oid, "is_deleted": { "$ne": true } };

        self.db
            .collection::<bson::Document>("skills")
            .update_many(filter.clone(), update.clone(), None)
            .await?;
        self.db
            .collection::<bson::Document>("recipes")
            .update_many(filter.clone(), update.clone(), None)
            .await?;
        self.db
            .collection::<bson::Document>("extensions")
            .update_many(filter.clone(), update.clone(), None)
            .await?;
        self.db
            .collection::<bson::Document>("documents")
            .update_many(filter, update, None)
            .await?;

        Ok(())
    }

    /// Update team
    pub async fn update(
        &self,
        team_id: &str,
        name: Option<String>,
        description: Option<String>,
        repository_url: Option<String>,
    ) -> Result<Team> {
        let oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Team>("teams");

        let mut update_doc = doc! { "updated_at": bson::DateTime::from_chrono(Utc::now()) };
        if let Some(n) = name {
            update_doc.insert("name", n);
        }
        if let Some(d) = description {
            update_doc.insert("description", d);
        }
        if let Some(r) = repository_url {
            update_doc.insert("repository_url", r);
        }

        coll.update_one(doc! { "_id": oid }, doc! { "$set": update_doc }, None)
            .await?;

        self.get(team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))
    }

    /// Get team settings
    pub async fn get_settings(&self, team_id: &str) -> Result<TeamSettings> {
        let team = self
            .get(team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))?;
        Ok(team.settings)
    }

    /// Update team settings (atomic $set on settings sub-document)
    pub async fn update_settings(&self, team_id: &str, settings: TeamSettings) -> Result<Team> {
        let oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Team>("teams");

        let settings_bson = bson::to_bson(&settings)?;
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$set": { "settings": settings_bson, "updated_at": bson::DateTime::from_chrono(Utc::now()) } },
            None,
        )
        .await?;

        self.get(team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Team not found"))
    }

    /// Add member to team
    pub async fn add_member(
        &self,
        team_id: &str,
        user_id: &str,
        display_name: &str,
        role: &str,
    ) -> Result<()> {
        let oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Team>("teams");
        let user_profile = self.load_active_user_profile(user_id).await?;
        let normalized_role = Self::normalize_member_role(role)?;
        let effective_display_name = if display_name.trim().is_empty() {
            if user_profile.display_name.trim().is_empty() {
                user_profile.email.clone()
            } else {
                user_profile.display_name.clone()
            }
        } else {
            display_name.trim().to_string()
        };

        let member = TeamMember {
            user_id: user_profile.user_id.clone(),
            email: user_profile.email,
            display_name: effective_display_name,
            role: normalized_role,
            status: "active".to_string(),
            permissions: MemberPermissions::default(),
            joined_at: Utc::now(),
        };

        let result = coll
            .update_one(
                doc! { "_id": oid, "members.user_id": { "$ne": user_profile.user_id } },
                doc! { "$push": { "members": bson::to_bson(&member)? } },
                None,
            )
            .await?;

        if result.matched_count == 0 {
            return Err(anyhow!("User is already a member of this team"));
        }

        Ok(())
    }

    /// Update member role
    pub async fn update_member_role(&self, team_id: &str, user_id: &str, role: &str) -> Result<()> {
        let oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Team>("teams");
        let normalized_role = Self::normalize_member_role(role)?;

        coll.update_one(
            doc! { "_id": oid, "members.user_id": user_id },
            doc! { "$set": { "members.$.role": normalized_role } },
            None,
        )
        .await?;

        Ok(())
    }

    /// Remove member from team
    pub async fn remove_member(&self, team_id: &str, user_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Team>("teams");

        coll.update_one(
            doc! { "_id": oid },
            doc! { "$pull": { "members": { "user_id": user_id } } },
            None,
        )
        .await?;

        Ok(())
    }

    /// List invites for a team
    pub async fn list_invites(&self, team_id: &str) -> Result<Vec<TeamInvite>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<TeamInvite>("team_invites");

        let cursor = coll.find(doc! { "team_id": team_oid }, None).await?;
        let invites: Vec<TeamInvite> = cursor.try_collect().await?;

        Ok(invites)
    }

    /// Create a new invite
    pub async fn create_invite(
        &self,
        team_id: &str,
        created_by: &str,
        invitee_email: Option<&str>,
        is_open_invite: bool,
        role: &str,
        expires_in_days: Option<i64>,
        max_uses: Option<i32>,
    ) -> Result<TeamInvite> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let normalized_role = Self::normalize_member_role(role)?;
        let (normalized_invitee_email, normalized_max_uses, expires_at) = if is_open_invite {
            if invitee_email
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false)
            {
                return Err(anyhow!("Open invite links must not bind a specific email"));
            }
            let days = Self::normalize_open_invite_days(expires_in_days)?;
            let normalized_max_uses = max_uses.unwrap_or(1).max(1);
            (
                "".to_string(),
                Some(normalized_max_uses),
                Some(now + Duration::days(days)),
            )
        } else {
            let email = Self::normalize_invitee_email(
                invitee_email.ok_or_else(|| anyhow!("Invite email is required"))?,
            )?;
            let expires_at = expires_in_days.map(|days| now + Duration::days(days));
            (email, Some(1), expires_at)
        };

        let invite = TeamInvite {
            id: None,
            team_id: team_oid,
            code: Uuid::new_v4().to_string(),
            role: normalized_role,
            invitee_email: normalized_invitee_email,
            is_open_invite,
            created_by: created_by.to_string(),
            expires_at,
            max_uses: normalized_max_uses,
            used_count: 0,
            created_at: now,
        };

        let coll = self.db.collection::<TeamInvite>("team_invites");
        let result = coll.insert_one(&invite, None).await?;

        let mut invite = invite;
        invite.id = result.inserted_id.as_object_id();
        Ok(invite)
    }

    /// Revoke an invite
    pub async fn revoke_invite(&self, team_id: &str, code: &str) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<TeamInvite>("team_invites");

        coll.delete_one(doc! { "team_id": team_oid, "code": code }, None)
            .await?;
        Ok(())
    }

    /// Get invite by code
    pub async fn get_invite_by_code(&self, code: &str) -> Result<Option<TeamInvite>> {
        let coll = self.db.collection::<TeamInvite>("team_invites");
        Ok(coll.find_one(doc! { "code": code }, None).await?)
    }

    /// Accept an invite and join the team
    pub async fn accept_invite(
        &self,
        code: &str,
        user_id: &str,
        display_name: &str,
    ) -> Result<crate::routes::mongo::teams::AcceptInviteResponse> {
        use crate::routes::mongo::teams::AcceptInviteResponse;

        let (invite, team) = match self.resolve_valid_invite(code).await {
            Ok(resolved) => resolved,
            Err(e) => {
                return Ok(AcceptInviteResponse {
                    success: false,
                    team_id: None,
                    team_name: None,
                    error: Some(e.to_string()),
                });
            }
        };
        let team_id = invite.team_id.to_hex();
        let user_profile = self.load_active_user_profile(user_id).await?;

        if !invite.is_open_invite {
            if let Err(e) =
                Self::ensure_invitee_email_matches(&invite.invitee_email, &user_profile.email)
            {
                return Ok(AcceptInviteResponse {
                    success: false,
                    team_id: Some(team_id),
                    team_name: Some(team.name),
                    error: Some(e.to_string()),
                });
            }
        }

        // Check if user is already a member
        if team.owner_id == user_id || team.members.iter().any(|m| m.user_id == user_id) {
            return Ok(AcceptInviteResponse {
                success: false,
                team_id: Some(team_id),
                team_name: Some(team.name),
                error: Some("You are already a member of this team".to_string()),
            });
        }

        let coll = self.db.collection::<TeamInvite>("team_invites");
        let invite_update_result = match invite.max_uses {
            Some(max_uses) => {
                coll.update_one(
                    doc! { "code": code, "used_count": { "$lt": max_uses } },
                    doc! { "$inc": { "used_count": 1 } },
                    None,
                )
                .await?
            }
            None => {
                coll.update_one(
                    doc! { "code": code },
                    doc! { "$inc": { "used_count": 1 } },
                    None,
                )
                .await?
            }
        };

        if invite_update_result.modified_count == 0 {
            return Ok(AcceptInviteResponse {
                success: false,
                team_id: Some(team_id),
                team_name: Some(team.name),
                error: Some("Invite has reached maximum uses".to_string()),
            });
        }

        // Add member to team
        if let Err(e) = self
            .add_member(&team_id, user_id, display_name, &invite.role)
            .await
        {
            let _ = coll
                .update_one(
                    doc! { "code": code },
                    doc! { "$inc": { "used_count": -1 } },
                    None,
                )
                .await;
            return Err(e);
        }

        Ok(AcceptInviteResponse {
            success: true,
            team_id: Some(team_id),
            team_name: Some(team.name),
            error: None,
        })
    }
}
