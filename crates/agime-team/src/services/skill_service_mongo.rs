//! Skill service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{
    increment_version, PaginatedResponse, ResourceNameValidator, Skill, SkillFile,
    SkillStorageType, SkillSummary,
};
use crate::services::package_service::{FrontmatterMetadata, PackageService, SkillFrontmatter};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId, Regex as BsonRegex};
use mongodb::options::FindOptions;

/// Generate SKILL.md content for an inline skill from its fields.
pub fn generate_skill_md_for_inline(
    name: &str,
    description: Option<&str>,
    content: &str,
    version: &str,
) -> String {
    let frontmatter = SkillFrontmatter {
        name: name.to_string(),
        description: description.unwrap_or("").to_string(),
        license: None,
        metadata: Some(FrontmatterMetadata {
            version: Some(version.to_string()),
            ..Default::default()
        }),
    };
    PackageService::generate_skill_md(&frontmatter, content)
}

pub struct SkillService {
    db: MongoDb,
}

impl SkillService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    /// Check if a skill name already exists in the team
    pub async fn check_duplicate_name(
        &self,
        team_id: &str,
        name: &str,
        exclude_id: Option<&str>,
    ) -> Result<bool> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Skill>("skills");
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

    /// Validate name, check duplicates, and insert a Skill document.
    async fn validate_and_insert(&self, team_id: &str, name: &str, skill: Skill) -> Result<Skill> {
        ResourceNameValidator::validate_kebab_case(name).map_err(|e| anyhow!(e))?;

        if self.check_duplicate_name(team_id, name, None).await? {
            return Err(anyhow!(
                "A skill with name '{}' already exists in this team",
                name
            ));
        }

        let coll = self.db.collection::<Skill>("skills");
        let result = coll.insert_one(&skill, None).await?;
        let mut skill = skill;
        skill.id = result.inserted_id.as_object_id();
        Ok(skill)
    }

    pub async fn create(
        &self,
        team_id: &str,
        user_id: &str,
        name: &str,
        content: &str,
        description: Option<String>,
        tags: Option<Vec<String>>,
        visibility: Option<String>,
    ) -> Result<Skill> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let version = "1.0.0".to_string();

        let skill_md = generate_skill_md_for_inline(
            name,
            description.as_deref(),
            content,
            &version,
        );

        let skill = Skill {
            id: None,
            team_id: team_oid,
            name: name.to_string(),
            description,
            storage_type: SkillStorageType::Inline,
            content: Some(content.to_string()),
            skill_md: Some(skill_md),
            files: vec![],
            manifest: None,
            package_url: None,
            package_hash: None,
            package_size: None,
            metadata: None,
            version,
            previous_version_id: None,
            tags: tags.unwrap_or_default(),
            dependencies: None,
            visibility: visibility.unwrap_or_else(|| "team".to_string()),
            protection_level: "team_installable".to_string(),
            use_count: 0,
            is_deleted: false,
            created_by: user_id.to_string(),
            ai_description: None,
            ai_description_lang: None,
            ai_described_at: None,
            created_at: now,
            updated_at: now,
        };

        self.validate_and_insert(team_id, name, skill).await
    }

    /// Create a skill in package mode with full SKILL.md and supporting files
    pub async fn create_package(
        &self,
        team_id: &str,
        user_id: &str,
        name: &str,
        description: Option<String>,
        skill_md: String,
        files: Vec<SkillFile>,
        body: String,
        tags: Option<Vec<String>>,
        visibility: Option<String>,
    ) -> Result<Skill> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();
        let version = "1.0.0".to_string();

        let skill = Skill {
            id: None,
            team_id: team_oid,
            name: name.to_string(),
            description,
            storage_type: SkillStorageType::Package,
            content: Some(body),
            skill_md: Some(skill_md),
            files,
            manifest: None,
            package_url: None,
            package_hash: None,
            package_size: None,
            metadata: None,
            version,
            previous_version_id: None,
            tags: tags.unwrap_or_default(),
            dependencies: None,
            visibility: visibility.unwrap_or_else(|| "team".to_string()),
            protection_level: "team_installable".to_string(),
            use_count: 0,
            is_deleted: false,
            created_by: user_id.to_string(),
            ai_description: None,
            ai_description_lang: None,
            ai_described_at: None,
            created_at: now,
            updated_at: now,
        };

        self.validate_and_insert(team_id, name, skill).await
    }

    /// List skills with pagination, search, and sorting
    pub async fn list(
        &self,
        team_id: &str,
        page: Option<u64>,
        limit: Option<u64>,
        search: Option<&str>,
        sort: Option<&str>,
    ) -> Result<PaginatedResponse<SkillSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Skill>("skills");

        let mut filter = doc! { "team_id": team_oid, "is_deleted": { "$ne": true } };

        // Apply search filter using $regex
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

        // Count total matching documents
        let total = coll.count_documents(filter.clone(), None).await?;

        let page = page.unwrap_or(1).max(1);
        let limit = limit.unwrap_or(100).min(1000);
        let skip = (page - 1) * limit;

        // Build sort document
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
        let skills: Vec<Skill> = cursor.try_collect().await?;
        let items: Vec<SkillSummary> = skills.into_iter().map(SkillSummary::from).collect();

        Ok(PaginatedResponse::new(items, total, page, limit))
    }

    pub async fn delete(&self, skill_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(skill_id)?;
        let coll = self.db.collection::<Skill>("skills");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$set": { "is_deleted": true, "updated_at": bson::DateTime::from_chrono(Utc::now()) } },
            None
        ).await?;
        Ok(())
    }

    pub async fn get(&self, skill_id: &str) -> Result<Option<Skill>> {
        let oid = ObjectId::parse_str(skill_id)?;
        let coll = self.db.collection::<Skill>("skills");
        Ok(coll
            .find_one(doc! { "_id": oid, "is_deleted": { "$ne": true } }, None)
            .await?)
    }

    pub async fn update(
        &self,
        skill_id: &str,
        name: Option<String>,
        description: Option<String>,
        content: Option<String>,
    ) -> Result<Skill> {
        let oid = ObjectId::parse_str(skill_id)?;
        let coll = self.db.collection::<Skill>("skills");

        // Get current skill for version tracking
        let current = self
            .get(skill_id)
            .await?
            .ok_or_else(|| anyhow!("Skill not found"))?;

        let mut set_doc = doc! { "updated_at": bson::DateTime::from_chrono(Utc::now()) };

        if let Some(ref n) = name {
            ResourceNameValidator::validate_kebab_case(n).map_err(|e| anyhow!(e))?;
            // Check duplicate name (excluding self)
            if self
                .check_duplicate_name(&current.team_id.to_hex(), n, Some(skill_id))
                .await?
            {
                return Err(anyhow!(
                    "A skill with name '{}' already exists in this team",
                    n
                ));
            }
            set_doc.insert("name", n.clone());
        }
        if let Some(ref d) = description {
            set_doc.insert("description", d.clone());
        }

        // Version tracking: if content changes, bump patch version
        let mut new_version = current.version.clone();
        if let Some(ref c) = content {
            set_doc.insert("content", c.clone());
            new_version = increment_version(&current.version);
            set_doc.insert("version", &new_version);
            set_doc.insert(
                "previous_version_id",
                current.id.map(|id| id.to_hex()).unwrap_or_default(),
            );
        }

        // Regenerate skill_md if any of name/description/content changed
        if name.is_some() || description.is_some() || content.is_some() {
            let final_name = name.as_deref().unwrap_or(&current.name);
            let final_desc = description
                .as_deref()
                .or(current.description.as_deref());
            let final_content = content
                .as_deref()
                .or(current.content.as_deref())
                .unwrap_or("");
            let skill_md =
                generate_skill_md_for_inline(final_name, final_desc, final_content, &new_version);
            set_doc.insert("skill_md", &skill_md);
        }

        coll.update_one(doc! { "_id": oid }, doc! { "$set": set_doc }, None)
            .await?;
        self.get(skill_id)
            .await?
            .ok_or_else(|| anyhow!("Skill not found"))
    }

    /// Update only the skill_md field (used by lazy-generation)
    pub async fn update_skill_md(&self, skill_id: &str, skill_md: &str) -> Result<()> {
        let oid = ObjectId::parse_str(skill_id)?;
        let coll = self.db.collection::<Skill>("skills");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$set": { "skill_md": skill_md } },
            None,
        )
        .await?;
        Ok(())
    }

    /// Atomically increment use_count
    pub async fn increment_use_count(&self, skill_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(skill_id)?;
        let coll = self.db.collection::<Skill>("skills");
        coll.update_one(
            doc! { "_id": oid },
            doc! { "$inc": { "use_count": 1 } },
            None,
        )
        .await?;
        Ok(())
    }

    /// Backfill skill_md for all inline skills in a team that have skill_md: null.
    /// Returns the number of skills updated.
    pub async fn backfill_skill_md(&self, team_id: &str) -> Result<u64> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Skill>("skills");
        let filter = doc! {
            "team_id": team_oid,
            "skill_md": null,
            "storage_type": "inline",
            "is_deleted": { "$ne": true }
        };
        let cursor = coll.find(filter, None).await?;
        let skills: Vec<Skill> = cursor.try_collect().await?;

        let mut count: u64 = 0;
        for skill in skills {
            let content = skill.content.as_deref().unwrap_or("");
            let skill_md = generate_skill_md_for_inline(
                &skill.name,
                skill.description.as_deref(),
                content,
                &skill.version,
            );
            let Some(oid) = skill.id else {
                tracing::warn!("Skill missing _id during backfill, skipping: {}", skill.name);
                continue;
            };
            coll.update_one(
                doc! { "_id": oid },
                doc! { "$set": { "skill_md": &skill_md } },
                None,
            )
            .await?;
            count += 1;
        }
        Ok(count)
    }
}
