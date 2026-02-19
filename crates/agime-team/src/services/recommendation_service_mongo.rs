//! Recommendation service for MongoDB

use crate::db::MongoDb;
use anyhow::Result;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Document as BsonDoc};
use mongodb::options::FindOptions;
use serde::Serialize;

/// Recommended item
#[derive(Debug, Clone, Serialize)]
pub struct RecommendedItem {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    #[serde(rename = "useCount")]
    pub use_count: i32,
    pub reason: String,
}

pub struct RecommendationService {
    db: MongoDb,
}

impl RecommendationService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Get popular resources (highest use_count)
    pub async fn get_popular(
        &self,
        team_id: &str,
        limit: Option<u64>,
    ) -> Result<Vec<RecommendedItem>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let limit = limit.unwrap_or(10).min(50);
        let filter = doc! {
            "team_id": &team_oid,
            "is_deleted": { "$ne": true },
            "use_count": { "$gt": 0 }
        };
        let options = FindOptions::builder()
            .limit(limit as i64)
            .sort(doc! { "use_count": -1 })
            .build();

        let mut items = Vec::new();
        self.collect_from("skills", "skill", "popular", &filter, &options, &mut items)
            .await?;
        self.collect_from(
            "recipes", "recipe", "popular", &filter, &options, &mut items,
        )
        .await?;
        self.collect_from(
            "extensions",
            "extension",
            "popular",
            &filter,
            &options,
            &mut items,
        )
        .await?;

        items.sort_by(|a, b| b.use_count.cmp(&a.use_count));
        items.truncate(limit as usize);
        Ok(items)
    }

    /// Get newest resources
    pub async fn get_newest(
        &self,
        team_id: &str,
        limit: Option<u64>,
    ) -> Result<Vec<RecommendedItem>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let limit = limit.unwrap_or(10).min(50);
        let filter = doc! {
            "team_id": &team_oid,
            "is_deleted": { "$ne": true }
        };
        let options = FindOptions::builder()
            .limit(limit as i64)
            .sort(doc! { "created_at": -1 })
            .build();

        let mut items = Vec::new();
        self.collect_from("skills", "skill", "new", &filter, &options, &mut items)
            .await?;
        self.collect_from("recipes", "recipe", "new", &filter, &options, &mut items)
            .await?;
        self.collect_from(
            "extensions",
            "extension",
            "new",
            &filter,
            &options,
            &mut items,
        )
        .await?;

        items.truncate(limit as usize);
        Ok(items)
    }

    /// Helper: collect items from a collection
    async fn collect_from(
        &self,
        collection: &str,
        resource_type: &str,
        reason: &str,
        filter: &mongodb::bson::Document,
        options: &FindOptions,
        items: &mut Vec<RecommendedItem>,
    ) -> Result<()> {
        let cursor = self
            .db
            .collection::<BsonDoc>(collection)
            .find(filter.clone(), options.clone())
            .await?;
        let docs: Vec<BsonDoc> = cursor.try_collect().await?;

        for d in docs {
            if let (Some(id), Some(name)) = (d.get_object_id("_id").ok(), d.get_str("name").ok()) {
                items.push(RecommendedItem {
                    id: id.to_hex(),
                    name: name.to_string(),
                    description: d.get_str("description").ok().map(|s| s.to_string()),
                    resource_type: resource_type.to_string(),
                    use_count: d.get_i32("use_count").unwrap_or(0),
                    reason: reason.to_string(),
                });
            }
        }
        Ok(())
    }
}
