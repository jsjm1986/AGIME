//! Stats service for MongoDB

use crate::db::MongoDb;
use anyhow::Result;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Document as BsonDoc};
use mongodb::options::FindOptions;
use serde::Serialize;

/// Resource statistics
#[derive(Debug, Clone, Serialize)]
pub struct ResourceStats {
    pub skills: u64,
    pub recipes: u64,
    pub extensions: u64,
    pub documents: u64,
    pub members: u64,
}

/// Trending item
#[derive(Debug, Clone, Serialize)]
pub struct TrendingItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "useCount")]
    pub use_count: i32,
}

pub struct StatsService {
    db: MongoDb,
}

impl StatsService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Get resource counts for a team
    pub async fn get_resource_stats(&self, team_id: &str) -> Result<ResourceStats> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let not_deleted = doc! { "team_id": &team_oid, "is_deleted": { "$ne": true } };

        let skills = self
            .db
            .collection::<BsonDoc>("skills")
            .count_documents(not_deleted.clone(), None)
            .await?;
        let recipes = self
            .db
            .collection::<BsonDoc>("recipes")
            .count_documents(not_deleted.clone(), None)
            .await?;
        let extensions = self
            .db
            .collection::<BsonDoc>("extensions")
            .count_documents(not_deleted.clone(), None)
            .await?;
        let documents = self
            .db
            .collection::<BsonDoc>("documents")
            .count_documents(not_deleted.clone(), None)
            .await?;

        // Count members from team document
        let team_coll = self.db.collection::<BsonDoc>("teams");
        let members =
            if let Some(team) = team_coll.find_one(doc! { "_id": &team_oid }, None).await? {
                team.get_array("members")
                    .map(|m| m.len() as u64)
                    .unwrap_or(0)
            } else {
                0
            };

        Ok(ResourceStats {
            skills,
            recipes,
            extensions,
            documents,
            members,
        })
    }

    /// Get trending resources (sorted by use_count desc)
    pub async fn get_trending(
        &self,
        team_id: &str,
        limit: Option<u64>,
    ) -> Result<Vec<TrendingItem>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let limit = limit.unwrap_or(10).min(50);
        let filter = doc! { "team_id": &team_oid, "is_deleted": { "$ne": true } };
        let options = FindOptions::builder()
            .limit(limit as i64)
            .sort(doc! { "use_count": -1 })
            .build();

        let mut items = Vec::new();

        // Skills
        let cursor = self
            .db
            .collection::<BsonDoc>("skills")
            .find(filter.clone(), options.clone())
            .await?;
        let docs: Vec<BsonDoc> = cursor.try_collect().await?;
        for d in docs {
            if let (Some(id), Some(name), Some(uc)) = (
                d.get_object_id("_id").ok(),
                d.get_str("name").ok(),
                d.get_i32("use_count").ok(),
            ) {
                if uc > 0 {
                    items.push(TrendingItem {
                        id: id.to_hex(),
                        name: name.to_string(),
                        resource_type: "skill".to_string(),
                        use_count: uc,
                    });
                }
            }
        }

        // Recipes
        let cursor = self
            .db
            .collection::<BsonDoc>("recipes")
            .find(filter.clone(), options.clone())
            .await?;
        let docs: Vec<BsonDoc> = cursor.try_collect().await?;
        for d in docs {
            if let (Some(id), Some(name), Some(uc)) = (
                d.get_object_id("_id").ok(),
                d.get_str("name").ok(),
                d.get_i32("use_count").ok(),
            ) {
                if uc > 0 {
                    items.push(TrendingItem {
                        id: id.to_hex(),
                        name: name.to_string(),
                        resource_type: "recipe".to_string(),
                        use_count: uc,
                    });
                }
            }
        }

        // Extensions
        let cursor = self
            .db
            .collection::<BsonDoc>("extensions")
            .find(filter.clone(), options.clone())
            .await?;
        let docs: Vec<BsonDoc> = cursor.try_collect().await?;
        for d in docs {
            if let (Some(id), Some(name), Some(uc)) = (
                d.get_object_id("_id").ok(),
                d.get_str("name").ok(),
                d.get_i32("use_count").ok(),
            ) {
                if uc > 0 {
                    items.push(TrendingItem {
                        id: id.to_hex(),
                        name: name.to_string(),
                        resource_type: "extension".to_string(),
                        use_count: uc,
                    });
                }
            }
        }

        // Sort all by use_count desc, take top N
        items.sort_by(|a, b| b.use_count.cmp(&a.use_count));
        items.truncate(limit as usize);
        Ok(items)
    }
}
