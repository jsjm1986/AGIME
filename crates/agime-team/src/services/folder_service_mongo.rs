//! Folder service for MongoDB

use crate::db::MongoDb;
use crate::models::mongo::{Folder, FolderSummary, FolderTreeNode};
use anyhow::{anyhow, Result};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{doc, oid::ObjectId};

pub struct FolderService {
    db: MongoDb,
}

impl FolderService {
    pub fn new(db: MongoDb) -> Self {
        Self { db }
    }

    pub async fn create(
        &self,
        team_id: &str,
        user_id: &str,
        name: &str,
        parent_path: &str,
    ) -> Result<Folder> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let now = Utc::now();

        // Build full path
        let full_path = if parent_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent_path, name)
        };

        // Check duplicate
        let coll = self.db.collection::<Folder>("folders");
        let exists = coll
            .count_documents(
                doc! {
                    "team_id": &team_oid,
                    "full_path": &full_path,
                    "is_deleted": { "$ne": true }
                },
                None,
            )
            .await?;

        if exists > 0 {
            return Err(anyhow!("Folder '{}' already exists", full_path));
        }

        let folder = Folder {
            id: None,
            team_id: team_oid,
            name: name.to_string(),
            parent_path: parent_path.to_string(),
            full_path,
            description: None,
            created_by: user_id.to_string(),
            is_deleted: false,
            is_system: false,
            created_at: now,
            updated_at: now,
        };

        let result = coll.insert_one(&folder, None).await?;
        let mut folder = folder;
        folder.id = result.inserted_id.as_object_id();
        Ok(folder)
    }

    pub async fn get(&self, folder_id: &str) -> Result<Option<Folder>> {
        let oid = ObjectId::parse_str(folder_id)?;
        let coll = self.db.collection::<Folder>("folders");
        Ok(coll
            .find_one(doc! { "_id": oid, "is_deleted": { "$ne": true } }, None)
            .await?)
    }

    pub async fn list(
        &self,
        team_id: &str,
        parent_path: Option<&str>,
    ) -> Result<Vec<FolderSummary>> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let coll = self.db.collection::<Folder>("folders");

        let mut filter = doc! {
            "team_id": team_oid,
            "is_deleted": { "$ne": true }
        };
        if let Some(path) = parent_path {
            filter.insert("parent_path", path);
        }

        let cursor = coll.find(filter, None).await?;
        let folders: Vec<Folder> = cursor.try_collect().await?;
        Ok(folders.into_iter().map(FolderSummary::from).collect())
    }

    pub async fn get_folder_tree(&self, team_id: &str) -> Result<Vec<FolderTreeNode>> {
        let all = self.list(team_id, None).await?;
        Ok(build_tree(&all, "/"))
    }

    pub async fn update(
        &self,
        folder_id: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<Folder> {
        let oid = ObjectId::parse_str(folder_id)?;
        let coll = self.db.collection::<Folder>("folders");

        // Reject renaming system folders
        if name.is_some() {
            if let Some(folder) = coll.find_one(doc! { "_id": oid, "is_deleted": { "$ne": true } }, None).await? {
                if folder.is_system {
                    return Err(anyhow!("Cannot rename a system folder"));
                }
            }
        }

        let mut set_doc = doc! {
            "updated_at": bson::DateTime::from_chrono(Utc::now())
        };
        if let Some(n) = name {
            set_doc.insert("name", n);
        }
        if let Some(d) = description {
            set_doc.insert("description", d);
        }

        coll.update_one(doc! { "_id": oid }, doc! { "$set": set_doc }, None)
            .await?;

        self.get(folder_id)
            .await?
            .ok_or_else(|| anyhow!("Folder not found"))
    }

    pub async fn delete(&self, folder_id: &str) -> Result<()> {
        let oid = ObjectId::parse_str(folder_id)?;
        let coll = self.db.collection::<Folder>("folders");

        // Reject deleting system folders
        if let Some(folder) = coll.find_one(doc! { "_id": oid, "is_deleted": { "$ne": true } }, None).await? {
            if folder.is_system {
                return Err(anyhow!("Cannot delete a system folder"));
            }
        }

        coll.update_one(
            doc! { "_id": oid },
            doc! { "$set": {
                "is_deleted": true,
                "updated_at": bson::DateTime::from_chrono(Utc::now())
            }},
            None,
        )
        .await?;
        Ok(())
    }

    /// Create a system folder if it doesn't exist (idempotent).
    pub async fn ensure_system_folder(
        &self,
        team_id: &str,
        name: &str,
        parent_path: &str,
    ) -> Result<()> {
        let team_oid = ObjectId::parse_str(team_id)?;
        let full_path = if parent_path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", parent_path, name)
        };

        let coll = self.db.collection::<Folder>("folders");
        let exists = coll
            .count_documents(
                doc! { "team_id": &team_oid, "full_path": &full_path, "is_deleted": { "$ne": true } },
                None,
            )
            .await?;

        if exists > 0 {
            return Ok(());
        }

        let now = Utc::now();
        let folder = Folder {
            id: None,
            team_id: team_oid,
            name: name.to_string(),
            parent_path: parent_path.to_string(),
            full_path,
            description: None,
            created_by: "system".to_string(),
            is_deleted: false,
            is_system: true,
            created_at: now,
            updated_at: now,
        };
        coll.insert_one(&folder, None).await?;
        Ok(())
    }
}

fn build_tree(folders: &[FolderSummary], parent: &str) -> Vec<FolderTreeNode> {
    folders
        .iter()
        .filter(|f| f.parent_path == parent)
        .map(|f| FolderTreeNode {
            id: f.id.clone(),
            name: f.name.clone(),
            full_path: f.full_path.clone(),
            is_system: f.is_system,
            children: build_tree(folders, &f.full_path),
        })
        .collect()
}
