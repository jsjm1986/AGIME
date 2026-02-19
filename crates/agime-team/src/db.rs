//! MongoDB database connection and configuration

use mongodb::bson::doc;
use mongodb::{options::ClientOptions, options::IndexOptions, Client, Database, IndexModel};

/// MongoDB database wrapper
#[derive(Clone)]
pub struct MongoDb {
    #[allow(dead_code)]
    client: Client,
    db: Database,
}

impl MongoDb {
    /// Connect to MongoDB
    pub async fn connect(uri: &str, db_name: &str) -> anyhow::Result<Self> {
        let options = ClientOptions::parse(uri).await?;
        let client = Client::with_options(options)?;
        let db = client.database(db_name);

        // Test connection
        db.run_command(doc! { "ping": 1 }, None).await?;
        tracing::info!("Connected to MongoDB: {}", db_name);

        let instance = Self { client, db };

        // Ensure indexes exist
        instance.ensure_indexes().await?;

        Ok(instance)
    }

    /// Get database reference
    pub fn db(&self) -> &Database {
        &self.db
    }

    /// Get collection
    pub fn collection<T>(&self, name: &str) -> mongodb::Collection<T> {
        self.db.collection(name)
    }

    /// Ping the database to check connection
    pub async fn ping(&self) -> anyhow::Result<()> {
        self.db
            .run_command(mongodb::bson::doc! { "ping": 1 }, None)
            .await?;
        Ok(())
    }

    /// Ensure all required indexes exist
    pub async fn ensure_indexes(&self) -> anyhow::Result<()> {
        tracing::info!("Ensuring MongoDB indexes...");

        // Teams collection indexes
        self.create_indexes(
            collections::TEAMS,
            vec![
                IndexModel::builder().keys(doc! { "owner_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "members.user_id": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": -1 })
                    .build(),
            ],
        )
        .await?;

        // Skills collection indexes
        self.create_indexes(
            collections::SKILLS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "name": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "created_by": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": -1 })
                    .build(),
                IndexModel::builder().keys(doc! { "is_deleted": 1 }).build(),
            ],
        )
        .await?;

        // Recipes collection indexes
        self.create_indexes(
            collections::RECIPES,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "name": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "category": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": -1 })
                    .build(),
                IndexModel::builder().keys(doc! { "is_deleted": 1 }).build(),
            ],
        )
        .await?;

        // Extensions collection indexes
        self.create_indexes(
            collections::EXTENSIONS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "name": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "extension_type": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": -1 })
                    .build(),
                IndexModel::builder().keys(doc! { "is_deleted": 1 }).build(),
            ],
        )
        .await?;

        // Documents collection indexes
        self.create_indexes(
            collections::DOCUMENTS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "folder_path": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "uploaded_by": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": -1 })
                    .build(),
                IndexModel::builder().keys(doc! { "is_deleted": 1 }).build(),
            ],
        )
        .await?;

        // Archived documents collection indexes
        self.create_indexes(
            collections::ARCHIVED_DOCUMENTS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "original_id": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "deleted_at": -1 })
                    .build(),
            ],
        )
        .await?;

        // Document versions collection indexes
        self.create_indexes(
            collections::DOCUMENT_VERSIONS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "document_id": 1, "version_number": -1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "document_id": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": -1 })
                    .build(),
            ],
        )
        .await?;

        // Document locks collection indexes
        self.create_indexes(
            collections::DOCUMENT_LOCKS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "document_id": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "expires_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::from_secs(0))
                            .build(),
                    )
                    .build(),
            ],
        )
        .await?;

        // Folders collection indexes
        self.create_indexes(
            collections::FOLDERS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "full_path": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "parent_path": 1 })
                    .build(),
                IndexModel::builder().keys(doc! { "is_deleted": 1 }).build(),
            ],
        )
        .await?;

        // Audit logs collection indexes (with 180-day TTL)
        self.create_indexes(
            collections::AUDIT_LOGS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "action": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "resource_type": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "user_id": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::from_secs(180 * 24 * 3600))
                            .build(),
                    )
                    .build(),
            ],
        )
        .await?;

        // Invites collection indexes
        self.create_indexes(
            collections::INVITES,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "code": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder().keys(doc! { "expires_at": 1 }).build(),
            ],
        )
        .await?;

        // User groups collection indexes
        self.create_indexes(
            collections::USER_GROUPS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "name": 1 })
                    .build(),
                IndexModel::builder().keys(doc! { "members": 1 }).build(),
                IndexModel::builder().keys(doc! { "is_deleted": 1 }).build(),
            ],
        )
        .await?;

        // Team agents collection indexes
        self.create_indexes(
            collections::TEAM_AGENTS,
            vec![
                IndexModel::builder().keys(doc! { "team_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "agent_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": -1 })
                    .build(),
            ],
        )
        .await?;

        // Users collection indexes
        self.create_indexes(
            collections::USERS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "user_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "email": 1 })
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! { "is_active": true })
                            .build(),
                    )
                    .build(),
            ],
        )
        .await?;

        // API keys collection indexes
        self.create_indexes(
            collections::API_KEYS,
            vec![
                IndexModel::builder().keys(doc! { "key_prefix": 1 }).build(),
                IndexModel::builder().keys(doc! { "user_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "key_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder().keys(doc! { "expires_at": 1 }).build(),
            ],
        )
        .await?;

        // Sessions collection indexes (with TTL for auto-cleanup)
        self.create_indexes(
            collections::SESSIONS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "session_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder().keys(doc! { "user_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "expires_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::from_secs(0))
                            .build(),
                    )
                    .build(),
            ],
        )
        .await?;

        // Registration requests collection indexes
        self.create_indexes(
            collections::REGISTRATION_REQUESTS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "request_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "email": 1, "status": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "status": 1, "created_at": -1 })
                    .build(),
            ],
        )
        .await?;

        // Auth audit logs collection indexes (with 90-day TTL)
        self.create_indexes(
            collections::AUTH_AUDIT_LOGS,
            vec![
                IndexModel::builder().keys(doc! { "action": 1 }).build(),
                IndexModel::builder().keys(doc! { "user_id": 1 }).build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::from_secs(90 * 24 * 3600))
                            .build(),
                    )
                    .build(),
            ],
        )
        .await?;

        // Smart logs collection indexes
        self.create_indexes(
            collections::SMART_LOGS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "created_at": -1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "resource_type": 1, "created_at": -1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::from_secs(90 * 24 * 3600))
                            .build(),
                    )
                    .build(),
            ],
        )
        .await?;

        // Portals collection indexes
        self.create_indexes(
            collections::PORTALS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "slug": 1 })
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! { "is_deleted": { "$eq": false } })
                            .build(),
                    )
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "status": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1, "created_at": -1 })
                    .build(),
                IndexModel::builder().keys(doc! { "is_deleted": 1 }).build(),
            ],
        )
        .await?;

        // Portal interactions collection indexes (with 90-day TTL)
        self.create_indexes(
            collections::PORTAL_INTERACTIONS,
            vec![
                IndexModel::builder()
                    .keys(doc! { "portal_id": 1, "created_at": -1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "team_id": 1 })
                    .build(),
                IndexModel::builder()
                    .keys(doc! { "created_at": 1 })
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::from_secs(90 * 24 * 3600))
                            .build(),
                    )
                    .build(),
            ],
        )
        .await?;

        tracing::info!("MongoDB indexes ensured successfully");
        Ok(())
    }

    /// Helper to create indexes for a collection
    async fn create_indexes(
        &self,
        collection: &str,
        indexes: Vec<IndexModel>,
    ) -> anyhow::Result<()> {
        let coll = self.db.collection::<mongodb::bson::Document>(collection);
        coll.create_indexes(indexes, None).await?;
        Ok(())
    }
}

/// Collection names
pub mod collections {
    pub const TEAMS: &str = "teams";
    pub const SKILLS: &str = "skills";
    pub const RECIPES: &str = "recipes";
    pub const EXTENSIONS: &str = "extensions";
    pub const DOCUMENTS: &str = "documents";
    pub const DOCUMENT_VERSIONS: &str = "document_versions";
    pub const DOCUMENT_LOCKS: &str = "document_locks";
    pub const FOLDERS: &str = "folders";
    pub const AUDIT_LOGS: &str = "audit_logs";
    pub const INVITES: &str = "invites";
    pub const USER_GROUPS: &str = "user_groups";
    pub const TEAM_AGENTS: &str = "team_agents";
    pub const USERS: &str = "users";
    pub const API_KEYS: &str = "api_keys";
    pub const SESSIONS: &str = "sessions";
    pub const REGISTRATION_REQUESTS: &str = "registration_requests";
    pub const AUTH_AUDIT_LOGS: &str = "auth_audit_logs";
    pub const SMART_LOGS: &str = "smart_logs";
    pub const ARCHIVED_DOCUMENTS: &str = "archived_documents";
    pub const PORTALS: &str = "portals";
    pub const PORTAL_INTERACTIONS: &str = "portal_interactions";
}
