//! Recipes HTTP routes

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::error::TeamError;
use crate::models::{
    SharedRecipe, ShareRecipeRequest, UpdateRecipeRequest, ListRecipesQuery,
    ResourceType, Dependency,
};
use crate::services::{RecipeService, InstallService};
use crate::routes::teams::TeamState;
use crate::routes::skills::{DependencyApiRequest, InstallResponse};

/// Query params for listing recipes
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListRecipesParams {
    pub team_id: Option<String>,
    pub search: Option<String>,
    pub category: Option<String>,
    pub author_id: Option<String>,
    pub tags: Option<String>,
    pub page: Option<u32>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
}

/// Share recipe request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareRecipeApiRequest {
    pub team_id: String,
    pub name: String,
    pub content_yaml: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
    pub dependencies: Option<Vec<DependencyApiRequest>>,
    pub protection_level: Option<String>,
}

/// Update recipe request (API)
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRecipeApiRequest {
    pub content_yaml: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
    pub protection_level: Option<String>,
}

/// Recipe response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeResponse {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    pub content_yaml: String,
    pub category: Option<String>,
    pub author_id: String,
    pub version: String,
    pub visibility: String,
    pub protection_level: String,
    pub tags: Vec<String>,
    pub use_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// Paginated recipes response
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipesListResponse {
    pub recipes: Vec<RecipeResponse>,
    pub total: u64,
    pub page: u32,
    pub limit: u32,
}

impl From<SharedRecipe> for RecipeResponse {
    fn from(recipe: SharedRecipe) -> Self {
        Self {
            id: recipe.id,
            team_id: recipe.team_id,
            name: recipe.name,
            description: recipe.description,
            content_yaml: recipe.content_yaml,
            category: recipe.category,
            author_id: recipe.author_id,
            version: recipe.version,
            visibility: recipe.visibility.to_string(),
            protection_level: recipe.protection_level.to_string(),
            tags: recipe.tags,
            use_count: recipe.use_count,
            created_at: recipe.created_at.to_rfc3339(),
            updated_at: recipe.updated_at.to_rfc3339(),
        }
    }
}

/// Configure recipes routes
pub fn routes(state: TeamState) -> Router {
    Router::new()
        .route("/recipes", post(share_recipe).get(list_recipes))
        .route("/recipes/{id}", get(get_recipe).put(update_recipe).delete(delete_recipe))
        .route("/recipes/{id}/install", post(install_recipe))
        .route("/recipes/{id}/uninstall", delete(uninstall_recipe))
        .with_state(state)
}

/// Share a recipe to a team
async fn share_recipe(
    State(state): State<TeamState>,
    Json(req): Json<ShareRecipeApiRequest>,
) -> Result<(StatusCode, Json<RecipeResponse>), TeamError> {
    let service = RecipeService::new();

    let dependencies = req.dependencies.map(|deps| {
        deps.into_iter()
            .map(|d| Dependency {
                dep_type: d.resource_type.parse().unwrap_or(ResourceType::Skill),
                name: d.name,
                version: d.version,
            })
            .collect()
    });

    let request = ShareRecipeRequest {
        team_id: req.team_id,
        name: req.name,
        content_yaml: req.content_yaml,
        description: req.description,
        category: req.category,
        tags: req.tags,
        visibility: req.visibility.and_then(|v| v.parse().ok()),
        dependencies,
        protection_level: req.protection_level.and_then(|p| p.parse().ok()),
    };

    let recipe = service.share_recipe(&state.pool, request, &state.user_id).await?;

    Ok((StatusCode::CREATED, Json(RecipeResponse::from(recipe))))
}

/// List recipes
async fn list_recipes(
    State(state): State<TeamState>,
    Query(params): Query<ListRecipesParams>,
) -> Result<Json<RecipesListResponse>, TeamError> {
    let service = RecipeService::new();

    let query = ListRecipesQuery {
        team_id: params.team_id,
        search: params.search,
        category: params.category,
        author_id: params.author_id,
        tags: params.tags.map(|t| t.split(',').map(|s| s.trim().to_string()).collect()),
        page: params.page.unwrap_or(1),
        limit: params.limit.unwrap_or(20).min(100),
        sort: params.sort.unwrap_or_else(|| "updated_at".to_string()),
    };

    let result = service.list_recipes(&state.pool, query, &state.user_id).await?;

    let response = RecipesListResponse {
        recipes: result.items.into_iter().map(RecipeResponse::from).collect(),
        total: result.total,
        page: result.page,
        limit: result.limit,
    };

    Ok(Json(response))
}

/// Get a recipe by ID
async fn get_recipe(
    State(state): State<TeamState>,
    Path(recipe_id): Path<String>,
) -> Result<Json<RecipeResponse>, TeamError> {
    let service = RecipeService::new();

    let recipe = service.get_recipe(&state.pool, &recipe_id).await?;

    Ok(Json(RecipeResponse::from(recipe)))
}

/// Update a recipe
async fn update_recipe(
    State(state): State<TeamState>,
    Path(recipe_id): Path<String>,
    Json(req): Json<UpdateRecipeApiRequest>,
) -> Result<Json<RecipeResponse>, TeamError> {
    let service = RecipeService::new();

    let request = UpdateRecipeRequest {
        content_yaml: req.content_yaml,
        description: req.description,
        category: req.category,
        tags: req.tags,
        visibility: req.visibility.and_then(|v| v.parse().ok()),
        dependencies: None,
        protection_level: req.protection_level.and_then(|p| p.parse().ok()),
    };

    let recipe = service.update_recipe(&state.pool, &recipe_id, request, &state.user_id).await?;

    Ok(Json(RecipeResponse::from(recipe)))
}

/// Delete a recipe
async fn delete_recipe(
    State(state): State<TeamState>,
    Path(recipe_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let service = RecipeService::new();

    service.delete_recipe(&state.pool, &recipe_id, &state.user_id).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Install a recipe
async fn install_recipe(
    State(state): State<TeamState>,
    Path(recipe_id): Path<String>,
) -> Result<Json<InstallResponse>, TeamError> {
    let service = InstallService::new();

    let result = service.install_resource(
        &state.pool,
        ResourceType::Recipe,
        &recipe_id,
        &state.user_id,
        &state.base_path,
    ).await?;

    Ok(Json(InstallResponse {
        success: result.success,
        resource_type: result.resource_type.to_string(),
        resource_id: result.resource_id,
        installed_version: Some(result.installed_version),
        local_path: result.local_path,
        error: result.error,
    }))
}

/// Uninstall a recipe
async fn uninstall_recipe(
    State(state): State<TeamState>,
    Path(recipe_id): Path<String>,
) -> Result<StatusCode, TeamError> {
    let service = InstallService::new();

    let result = service.uninstall_resource(
        &state.pool,
        ResourceType::Recipe,
        &recipe_id,
    ).await?;

    if result.success {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(TeamError::Internal(result.error.unwrap_or_else(|| "Uninstall failed".to_string())))
    }
}
