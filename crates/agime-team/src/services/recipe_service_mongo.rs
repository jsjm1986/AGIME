//! Recipe service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{
    increment_version, PaginatedResponse, Recipe, RecipeSummary, ResourceNameValidator,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Regex as BsonRegex};
use mongodb::options::FindOptions;

pub struct RecipeService {
    db: MongoDb,
}

impl RecipeService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    pub async fn check_duplicate_name(
        &self,
        team_id: &str,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<bool> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Recipe>("recipes");
        let mut filter = doc! {
            "team_id": team_oid,
            "name": name,
            "is_deleted": { "$ne": true }
        };
        if let Some(eid) = exclude_id {
            filter.insert("_id", doc! { "$ne": ObjectId::parse_str(eid)? });
        }
        let count = coll.count_documents(filter, None).await?;
        Ok(count > 0)
    }

    pub async fn create(
        &self,
        team_id: &str,
        user_id: &str,
        name: &str,
        content_yaml: &str,
        description: Option<String>,
        category: Option<String>,
        tags: Option<Vec<String>>,
        visibility: Option<String>,
    ) -> Result<Recipe> {
        ResourceNameValidator::validate(name).map_err(|e| anyhow!(e))?;

        if self.check_duplicate_name(team_id, name, None).await? {
            return Err(anyhow!(
                "A recipe with name '{}' already exists in this team",
                name
            ));
        }

        // Validate YAML syntax
        serde_yaml::from_str::<serde_yaml::Value>(content_yaml)
            .map_err(|e| anyhow!("Invalid YAML: {}", e))?;

        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();

        let recipe = Recipe {
            id: None,
            team_id: team_oid,
            name: name.to_string(),
            description,
            content_yaml: content_yaml.to_string(),
            category,
            tags: tags.unwrap_or_default(),
            version: "1.0.0".to_string(),
            previous_version_id: None,
            dependencies: None,
            visibility: visibility.unwrap_or_else(|| "team".to_string()),
            protection_level: "team_installable".to_string(),
            use_count: 0,
            is_deleted: false,
            created_by: user_id.to_string(),
            created_at: now,
            updated_at: now,
        };

        let coll = self.db.collection::<Recipe>("recipes");
        let result = coll.insert_one(&recipe, None).await?;

        let mut recipe = recipe;
        recipe.id = result.inserted_id.as_object_id();
        Ok(recipe)
    }

    pub async fn list(
        &self,
        team_id: &str,
        page: Option<u64>,
        limit: Option<u64>,
        search: Option<&str>,
        sort: Option<&str>,
    ) -> Result<PaginatedResponse<RecipeSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Recipe>("recipes");

        let mut filter = doc! { "team_id": team_oid, "is_deleted": { "$ne": true } };

        if let Some(q) = search {
            if !q.is_empty() {
                let escaped = regex::escape(q);
                let re = BsonRegex {
                    pattern: escaped,
                    options: "i".to_string(),
                };
                filter.insert(
                    "$or",
                    vec![
                        doc! { "name": { "$regex": &re } },
                        doc! { "description": { "$regex": &re } },
                    ],
                );
            }
        }

        let total = coll.count_documents(filter.clone(), None).await?;

        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(100).min(1000);
        let skip = (page - 1) * limit;

        let sort_doc = match sort.unwrap_or("updated_at") {
            "name" => doc! { "name": 1 },
            "created_at" => doc! { "created_at": -1 },
            "use_count" => doc! { "use_count": -1 },
            _ => doc! { "updated_at": -1 },
        };

        let options = FindOptions::builder()
            .skip(skip)
            .limit(limit as i64)
            .sort(sort_doc)
            .build();

        let cursor = coll.find(filter, options).await?;
        let recipes: Vec<Recipe> = cursor.try_collect().await?;
        let items: Vec<RecipeSummary> = recipes.into_iter().map(RecipeSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    pub async fn delete(&self, recipe_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(recipe_id)?;
        let coll = self.db.collection::<Recipe>("recipes");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$set": { "is_deleted": true, "updated_at": bson::DateTime::from_chrono(Utc::now()) } },
            None
        ).await?;
        Ok(())
    }

    pub async fn get(&self, recipe_id: &str) -> Result<Option<Recipe>> {
        let oid = ObjectId::parse_str(recipe_id)?;
        let coll = self.db.collection::<Recipe>("recipes");
        Ok(coll
            .find_one(doc! { "_id": oid, "is_deleted": { "$ne": true } }, None)
            .await?)
    }

    pub async fn update(
        &self,
        recipe_id: &str,
        name: Option<String>,
        description: Option<String>,
        content_yaml: Option<String>,
    ) -> Result<Recipe> {
        let oid = ObjectId::parse_str(recipe_id)?;
        let coll = self.db.collection::<Recipe>("recipes");

        let current = self
            .get(recipe_id)
            .await?
            .ok_or_else(|| anyhow!("Recipe not found"))?;

        let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(Utc::now()) };

        if let Some(ref n) = name {
            ResourceNameValidator::validate(n).map_err(|e| anyhow!(e))?;
            if self
                .check_duplicate_name(&current.team_id.to_hex(), n, Some(recipe_id))
                .await?
            {
                return Err(anyhow!(
                    "A recipe with name '{}' already exists in this team",
                    n
                ));
            }
            set_doc.insert("name", n.clone());
        }
        if let Some(d) = description {
            set_doc.insert("description", d);
        }

        if let Some(c) = content_yaml {
            // Validate YAML
            serde_yaml::from_str::<serde_yaml::Value>(&c)
                .map_err(|e| anyhow!("Invalid YAML: {}", e))?;
            set_doc.insert("content_yaml", c);
            let new_version = increment_version(&current.version);
            set_doc.insert("version", new_version);
            set_doc.insert(
                "previous_version_id",
                current.id.map(|id| id.to_hex()).unwrap_or_default(),
            );
        }

        coll.update_one(doc! { "_id": oid }, doc! { "$set": set_doc }, None)
            .await?;
        self.get(recipe_id)
            .await?
            .ok_or_else(|| anyhow!("Recipe not found"))
    }

    pub async fn increment_use_count(&self, recipe_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(recipe_id)?;
        let coll = self.db.collection::<Recipe>("recipes");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$inc": { "use_count": 1 } },
            None,
        )
        .await?;
        Ok(())
    }
}
