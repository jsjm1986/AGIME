//! Services module - business logic layer
//!
//! Services are currently placeholder implementations.
//! They define the interface for team operations and will be
//! fully implemented when integrated with the main application.

pub mod audit_service;
pub mod cleanup_service;
pub mod concurrency;
pub mod document_service;
pub mod extension_service;
pub mod install_service;
pub mod invite_service;
pub mod member_service;
pub mod package_service;
pub mod recipe_service;
pub mod recommendation_service;
pub mod skill_service;
pub mod stats_service;
pub mod team_service;

// MongoDB services
pub mod mongo;

pub use audit_service::{AuditAction, AuditLog, AuditLogQuery, AuditService};
pub use cleanup_service::{CleanupResult, CleanupService};
pub use concurrency::{ConcurrencyService, ConflictError, ETag};
pub use document_service::DocumentService;
pub use extension_service::ExtensionService;
pub use install_service::InstallService;
pub use invite_service::InviteService;
pub use member_service::{MemberService, RemoveMemberResult};
pub use package_service::{PackageService, ParsedSkillPackage, SkillFrontmatter};
pub use recipe_service::RecipeService;
pub use recommendation_service::{Recommendation, RecommendationRequest, RecommendationService};
pub use skill_service::SkillService;
pub use stats_service::{ActivityAction, ResourceStats, StatsService, TrendingResource};
pub use team_service::TeamService;

use sqlx::SqlitePool;
use std::sync::Arc;

/// Shared database pool type
pub type DbPool = Arc<SqlitePool>;
