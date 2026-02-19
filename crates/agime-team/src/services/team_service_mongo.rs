//! Team service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{
    CreateTeamRequest, MemberPermissions, Team, TeamInvite, TeamMember, TeamSettings, TeamSummary,
};
use anyhow::Result;
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};
use uuid::Uuid;

pub struct TeamService {
    db: MongoDb,
}

impl TeamService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Create a new team
    pub async fn create(&self, user_id: &str, req: CreateTeamRequest) -> Result<Team> {
        let now = Utc::now();
        let team = Team {
            id: None,
            name: req.name,
            description: req.description,
            repository_url: None,
            owner_id: user_id.to_string(),
            members: vec![TeamMember {
                user_id: user_id.to_string(),
                email: String::new(),
                display_name: String::new(),
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
        let coll = self.db.collection::<Team>("teams");
        // Filter out soft-deleted teams
        let filter = doc! { "members.user_id": user_id, "is_deleted": { "$ne": true } };

        let cursor = coll.find(filter, None).await?;
        let teams: Vec<Team> = cursor.try_collect().await?;

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

        let member = TeamMember {
            user_id: user_id.to_string(),
            email: user_id.to_string(),
            display_name: display_name.to_string(),
            role: role.to_string(),
            status: "active".to_string(),
            permissions: MemberPermissions::default(),
            joined_at: Utc::now(),
        };

        coll.update_one(
            doc! { "_id": oid },
            doc! { "$push": { "members": bson::to_bson(&member)? } },
            None,
        )
        .await?;

        Ok(())
    }

    /// Update member role
    pub async fn update_member_role(&self, team_id: &str, user_id: &str, role: &str) -> Result<()> {
        let oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Team>("teams");

        coll.update_one(
            doc! { "_id": oid, "members.user_id": user_id },
            doc! { "$set": { "members.$.role": role } },
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
        role: &str,
        expires_in_days: Option<i64>,
        max_uses: Option<i32>,
    ) -> Result<TeamInvite> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let expires_at = expires_in_days.map(|days| now + Duration::days(days));

        let invite = TeamInvite {
            id: None,
            team_id: team_oid,
            code: Uuid::new_v4().to_string(),
            role: role.to_string(),
            created_by: created_by.to_string(),
            expires_at,
            max_uses,
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

        // Get the invite
        let invite = match self.get_invite_by_code(code).await? {
            Some(i) => i,
            None => {
                return Ok(AcceptInviteResponse {
                    success: false,
                    team_id: None,
                    team_name: None,
                    error: Some("Invalid invite code".to_string()),
                })
            }
        };

        // Check expiration
        if let Some(expires_at) = invite.expires_at {
            if expires_at < Utc::now() {
                return Ok(AcceptInviteResponse {
                    success: false,
                    team_id: None,
                    team_name: None,
                    error: Some("Invite has expired".to_string()),
                });
            }
        }

        // Check max uses
        if let Some(max) = invite.max_uses {
            if invite.used_count >= max {
                return Ok(AcceptInviteResponse {
                    success: false,
                    team_id: None,
                    team_name: None,
                    error: Some("Invite has reached maximum uses".to_string()),
                });
            }
        }

        let team_id = invite.team_id.to_hex();

        // Check if team still exists (not soft-deleted)
        let team = match self.get(&team_id).await? {
            Some(t) => t,
            None => {
                return Ok(AcceptInviteResponse {
                    success: false,
                    team_id: None,
                    team_name: None,
                    error: Some("Team no longer exists".to_string()),
                })
            }
        };

        // Check if user is already a member
        if team.members.iter().any(|m| m.user_id == user_id) {
            return Ok(AcceptInviteResponse {
                success: false,
                team_id: Some(team_id),
                team_name: Some(team.name),
                error: Some("You are already a member of this team".to_string()),
            });
        }

        // Add member to team
        self.add_member(&team_id, user_id, display_name, &invite.role)
            .await?;

        // Increment used count
        let coll = self.db.collection::<TeamInvite>("team_invites");
        coll.update_one(
            doc! { "code": code },
            doc! { "$inc": { "used_count": 1 } },
            None,
        )
        .await?;

        Ok(AcceptInviteResponse {
            success: true,
            team_id: Some(team_id),
            team_name: Some(team.name),
            error: None,
        })
    }
}
