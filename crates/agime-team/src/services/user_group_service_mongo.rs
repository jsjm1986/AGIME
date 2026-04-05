//! User Group service for MongoDB
//! Manages user groups for team-level access control

use crate::db::MongoDb;
use crate::models::mongo::user_group_mongo::*;
use crate::models::mongo::Team;
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Document as BsonDoc};
use mongodb::options::FindOptions;
use std::collections::{HashMap, HashSet};

pub struct UserGroupService {
    db: MongoDb,
}

impl UserGroupService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    fn collection(&self) -> mongodb::Collection<BsonDoc> {
        self.db.collection("user_groups")
    }

    async fn load_team_member_directory(
        &self,
        team_id: &str,
    ) -> Result<HashMap<String, UserGroupMemberDetail>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let team = self
            .db
            .collection::<Team>("teams")
            .find_one(
                doc! {
                    "_id": &team_oid,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?
            .ok_or_else(|| anyhow!("Team not found"))?;

        let mut directory = HashMap::new();
        let mut owner_present = false;

        for member in team.members {
            if member.user_id == team.owner_id {
                owner_present = true;
            }

            let display_name = if member.display_name.trim().is_empty() {
                member.email.clone()
            } else {
                member.display_name.clone()
            };

            directory.insert(
                member.user_id.clone(),
                UserGroupMemberDetail {
                    user_id: member.user_id,
                    display_name,
                    email: member.email,
                    role: member.role,
                },
            );
        }

        if !owner_present {
            if let Some(owner) = self
                .db
                .collection::<BsonDoc>("users")
                .find_one(
                    doc! {
                        "user_id": &team.owner_id,
                        "is_active": true
                    },
                    None,
                )
                .await?
            {
                let email = owner.get_str("email").unwrap_or("").to_string();
                let display_name = owner
                    .get_str("display_name")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(email.as_str())
                    .to_string();

                directory.insert(
                    team.owner_id.clone(),
                    UserGroupMemberDetail {
                        user_id: team.owner_id,
                        display_name,
                        email,
                        role: "owner".to_string(),
                    },
                );
            }
        }

        Ok(directory)
    }

    async fn normalize_member_ids(
        &self,
        team_id: &str,
        member_ids: &[String],
    ) -> Result<Vec<String>> {
        let directory = self.load_team_member_directory(team_id).await?;
        let mut seen = HashSet::new();
        let mut normalized = Vec::new();

        for member_id in member_ids {
            let candidate = member_id.trim();
            if candidate.is_empty() {
                continue;
            }
            if !directory.contains_key(candidate) {
                return Err(anyhow!("User '{}' is not a member of this team", candidate));
            }
            if seen.insert(candidate.to_string()) {
                normalized.push(candidate.to_string());
            }
        }

        Ok(normalized)
    }

    /// Create a new user group
    pub async fn create(
        &self,
        team_id: &str,
        user_id: &str,
        req: CreateUserGroupRequest,
    ) -> Result<UserGroupDetail> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = bson::DateTime::from_chrono(Utc::now());
        let members = self.normalize_member_ids(team_id, &req.members).await?;

        // Check duplicate name
        let existing = self
            .collection()
            .find_one(
                doc! {
                    "team_id": &team_oid,
                    "name": &req.name,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?;

        if existing.is_some() {
            anyhow::bail!("Group name '{}' already exists", req.name);
        }

        let doc = doc! {
            "team_id": &team_oid,
            "name": &req.name,
            "description": req.description.as_deref(),
            "members": &members,
            "color": req.color.as_deref(),
            "is_system": false,
            "is_deleted": false,
            "created_by": user_id,
            "created_at": &now,
            "updated_at": &now,
        };

        let result = self.collection().insert_one(doc, None).await?;
        let id = result
            .inserted_id
            .as_object_id()
            .ok_or_else(|| anyhow::anyhow!("Failed to get inserted id"))?;

        self.get(team_id, &id.to_hex())
            .await?
            .ok_or_else(|| anyhow::anyhow!("Failed to retrieve created group"))
    }

    /// Get a user group by ID
    pub async fn get(&self, team_id: &str, group_id: &str) -> Result<Option<UserGroupDetail>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let group_oid = ObjectId::parse_str(group_id)?;
        let member_directory = self.load_team_member_directory(team_id).await?;

        let doc = self
            .collection()
            .find_one(
                doc! {
                    "_id": &group_oid,
                    "team_id": &team_oid,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?;

        Ok(doc.map(|d| doc_to_detail(&d, &member_directory)))
    }

    /// List user groups for a team
    pub async fn list(
        &self,
        team_id: &str,
        page: u32,
        limit: u32,
    ) -> Result<(Vec<UserGroupSummary>, u64)> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let member_directory = self.load_team_member_directory(team_id).await?;
        let filter = doc! {
            "team_id": &team_oid,
            "is_deleted": { "$ne": true }
        };

        let total = self
            .collection()
            .count_documents(filter.clone(), None)
            .await?;

        let skip = ((page.saturating_sub(1)) * limit) as u64;
        let options = FindOptions::builder()
            .sort(doc! { "name": 1 })
            .skip(skip)
            .limit(limit as i64)
            .build();

        let cursor = self.collection().find(filter, options).await?;
        let docs: Vec<BsonDoc> = cursor.try_collect().await?;
        let items = docs
            .iter()
            .map(|doc| doc_to_summary(doc, &member_directory))
            .collect();

        Ok((items, total))
    }

    /// Update a user group
    pub async fn update(
        &self,
        team_id: &str,
        group_id: &str,
        req: UpdateUserGroupRequest,
    ) -> Result<Option<UserGroupDetail>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let group_oid = ObjectId::parse_str(group_id)?;
        let now = bson::DateTime::from_chrono(Utc::now());

        let mut set = doc! { "updated_at": &now };
        if let Some(name) = &req.name {
            // Check duplicate name (excluding self)
            let existing = self
                .collection()
                .find_one(
                    doc! {
                        "team_id": &team_oid,
                        "name": name,
                        "_id": { "$ne": &group_oid },
                        "is_deleted": { "$ne": true }
                    },
                    None,
                )
                .await?;
            if existing.is_some() {
                anyhow::bail!("Group name '{}' already exists", name);
            }
            set.insert("name", name.as_str());
        }
        if let Some(desc) = &req.description {
            set.insert("description", desc.as_str());
        }
        if let Some(color) = &req.color {
            set.insert("color", color.as_str());
        }

        let result = self
            .collection()
            .update_one(
                doc! {
                    "_id": &group_oid,
                    "team_id": &team_oid,
                    "is_deleted": { "$ne": true },
                    "is_system": { "$ne": true }
                },
                doc! { "$set": set },
                None,
            )
            .await?;

        if result.modified_count == 0 {
            return Ok(None);
        }

        self.get(team_id, group_id).await
    }

    /// Update group members (add/remove)
    pub async fn update_members(
        &self,
        team_id: &str,
        group_id: &str,
        req: UpdateGroupMembersRequest,
    ) -> Result<Option<UserGroupDetail>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let group_oid = ObjectId::parse_str(group_id)?;
        let now = bson::DateTime::from_chrono(Utc::now());
        let normalized_add = self.normalize_member_ids(team_id, &req.add).await?;
        let normalized_remove: Vec<String> = req
            .remove
            .iter()
            .map(|member_id| member_id.trim().to_string())
            .filter(|member_id| !member_id.is_empty())
            .collect();

        let filter = doc! {
            "_id": &group_oid,
            "team_id": &team_oid,
            "is_deleted": { "$ne": true }
        };

        // Add members
        if !normalized_add.is_empty() {
            self.collection()
                .update_one(
                    filter.clone(),
                    doc! { "$addToSet": { "members": { "$each": &normalized_add } } },
                    None,
                )
                .await?;
        }

        // Remove members
        if !normalized_remove.is_empty() {
            self.collection()
                .update_one(
                    filter.clone(),
                    doc! { "$pull": { "members": { "$in": &normalized_remove } } },
                    None,
                )
                .await?;
        }

        // Update timestamp
        self.collection()
            .update_one(filter, doc! { "$set": { "updated_at": &now } }, None)
            .await?;

        self.get(team_id, group_id).await
    }

    /// Soft delete a user group
    pub async fn delete(&self, team_id: &str, group_id: &str) -> Result<bool> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let group_oid = ObjectId::parse_str(group_id)?;
        let now = bson::DateTime::from_chrono(Utc::now());

        let result = self
            .collection()
            .update_one(
                doc! {
                    "_id": &group_oid,
                    "team_id": &team_oid,
                    "is_deleted": { "$ne": true },
                    "is_system": { "$ne": true }
                },
                doc! { "$set": { "is_deleted": true, "updated_at": &now } },
                None,
            )
            .await?;

        Ok(result.modified_count > 0)
    }

    /// Get all group IDs a user belongs to (for access control checks)
    pub async fn get_user_group_ids(&self, team_id: &str, user_id: &str) -> Result<Vec<String>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let filter = doc! {
            "team_id": &team_oid,
            "members": user_id,
            "is_deleted": { "$ne": true }
        };

        let options = FindOptions::builder().projection(doc! { "_id": 1 }).build();

        let cursor = self.collection().find(filter, options).await?;
        let docs: Vec<BsonDoc> = cursor.try_collect().await?;

        Ok(docs
            .iter()
            .filter_map(|d| d.get_object_id("_id").ok().map(|oid| oid.to_hex()))
            .collect())
    }
}

/// Convert BSON document to UserGroupSummary
fn doc_to_summary(
    d: &BsonDoc,
    member_directory: &HashMap<String, UserGroupMemberDetail>,
) -> UserGroupSummary {
    let members = d
        .get_array("members")
        .map(|a| {
            a.iter()
                .filter_map(|value| value.as_str())
                .filter(|member_id| member_directory.contains_key(*member_id))
                .count()
        })
        .unwrap_or(0);
    UserGroupSummary {
        id: d
            .get_object_id("_id")
            .map(|o| o.to_hex())
            .unwrap_or_default(),
        name: d.get_str("name").unwrap_or("").to_string(),
        description: d.get_str("description").ok().map(|s| s.to_string()),
        member_count: members,
        color: d.get_str("color").ok().map(|s| s.to_string()),
        is_system: d.get_bool("is_system").unwrap_or(false),
        created_at: d
            .get_datetime("created_at")
            .map(|dt| dt.to_chrono())
            .unwrap_or_else(|_| Utc::now()),
        updated_at: d
            .get_datetime("updated_at")
            .map(|dt| dt.to_chrono())
            .unwrap_or_else(|_| Utc::now()),
    }
}

/// Convert BSON document to UserGroupDetail
fn doc_to_detail(
    d: &BsonDoc,
    member_directory: &HashMap<String, UserGroupMemberDetail>,
) -> UserGroupDetail {
    let members: Vec<String> = d
        .get_array("members")
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .filter(|member_id| member_directory.contains_key(*member_id))
                .map(|member_id| member_id.to_string())
                .collect()
        })
        .unwrap_or_default();
    let member_details = members
        .iter()
        .filter_map(|member_id| member_directory.get(member_id).cloned())
        .collect();
    UserGroupDetail {
        id: d
            .get_object_id("_id")
            .map(|o| o.to_hex())
            .unwrap_or_default(),
        name: d.get_str("name").unwrap_or("").to_string(),
        description: d.get_str("description").ok().map(|s| s.to_string()),
        members,
        member_details,
        color: d.get_str("color").ok().map(|s| s.to_string()),
        is_system: d.get_bool("is_system").unwrap_or(false),
        created_by: d.get_str("created_by").unwrap_or("").to_string(),
        created_at: d
            .get_datetime("created_at")
            .map(|dt| dt.to_chrono())
            .unwrap_or_else(|_| Utc::now()),
        updated_at: d
            .get_datetime("updated_at")
            .map(|dt| dt.to_chrono())
            .unwrap_or_else(|_| Utc::now()),
    }
}
