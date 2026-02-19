//! MongoDB routes - Recipes API

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::teams::{can_manage_team, is_team_member, AppState};
use super::InstallResponse;
use crate::models::mongo::SmartLogContext;
use crate::services::mongo::{RecipeService, TeamService};
use crate::AuthenticatedUserId;

#[derive(Debug, Deserialize)]
pub struct RecipeQuery {
    #[serde(rename = "teamId")]
    pub team_id: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
    pub search: Option<String>,
    pub sort: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RecipesResponse {
    pub recipes: Vec<RecipeInfo>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    #[serde(rename = "totalPages")]
    pub total_pages: u64,
}

/// Recipe info matching frontend SharedRecipe interface
#[derive(Debug, Serialize)]
pub struct RecipeInfo {
    pub id: String,
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "contentYaml")]
    pub content_yaml: String,
    pub category: Option<String>,
    #[serde(rename = "authorId")]
    pub author_id: String,
    pub version: String,
    pub visibility: String,
    #[serde(rename = "protectionLevel")]
    pub protection_level: String,
    pub tags: Vec<String>,
    #[serde(rename = "useCount")]
    pub use_count: i32,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateRecipeRequest {
    #[serde(rename = "teamId")]
    pub team_id: String,
    pub name: String,
    #[serde(rename = "contentYaml")]
    pub content_yaml: String,
    pub description: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRecipeRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "contentYaml")]
    pub content_yaml: Option<String>,
}

pub fn recipe_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/recipes", get(list_recipes).post(create_recipe))
        .route(
            "/recipes/{id}",
            get(get_recipe).put(update_recipe).delete(delete_recipe),
        )
        .route("/recipes/{id}/install", post(install_recipe))
        .route("/recipes/{id}/uninstall", delete(uninstall_recipe))
}

fn recipe_to_json(recipe: crate::models::mongo::Recipe) -> serde_json::Value {
    serde_json::json!({
        "id": recipe.id.map(|id| id.to_hex()).unwrap_or_default(),
        "teamId": recipe.team_id.to_hex(),
        "name": recipe.name,
        "description": recipe.description,
        "contentYaml": recipe.content_yaml,
        "category": recipe.category,
        "authorId": recipe.created_by,
        "version": recipe.version,
        "previousVersionId": recipe.previous_version_id,
        "visibility": recipe.visibility,
        "protectionLevel": recipe.protection_level,
        "dependencies": recipe.dependencies,
        "tags": recipe.tags,
        "useCount": recipe.use_count,
        "createdAt": recipe.created_at.to_rfc3339(),
        "updatedAt": recipe.updated_at.to_rfc3339()
    })
}

async fn list_recipes(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Query(query): Query<RecipeQuery>,
) -> Result<Json<RecipesResponse>, (StatusCode, String)> {
    let team_id = query
        .team_id
        .ok_or((StatusCode::BAD_REQUEST, "teamId required".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view recipes".to_string(),
        ));
    }

    let service = RecipeService::new((*state.db).clone());
    let result = service
        .list(
            &team_id,
            query.page,
            query.limit,
            query.search.as_deref(),
            query.sort.as_deref(),
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let recipes: Vec<RecipeInfo> = result
        .items
        .into_iter()
        .map(|r| RecipeInfo {
            id: r.id,
            team_id: r.team_id,
            name: r.name,
            description: r.description,
            content_yaml: r.content_yaml,
            category: r.category,
            author_id: r.author_id,
            version: r.version,
            visibility: r.visibility,
            protection_level: r.protection_level,
            tags: r.tags,
            use_count: r.use_count,
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(RecipesResponse {
        recipes,
        total: result.total,
        page: result.page,
        limit: result.limit,
        total_pages: result.total_pages,
    }))
}

async fn create_recipe(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Json(req): Json<CreateRecipeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&req.team_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can create recipes".to_string(),
        ));
    }

    let service = RecipeService::new((*state.db).clone());
    let recipe = service
        .create(
            &req.team_id,
            &user.0,
            &req.name,
            &req.content_yaml,
            req.description.clone(),
            req.category,
            req.tags,
            req.visibility,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            req.team_id.clone(),
            user.0.clone(),
            "create",
            "recipe",
            recipe.id.map(|id| id.to_hex()).unwrap_or_default(),
            recipe.name.clone(),
            Some(format!(
                "名称: {}\n描述: {}\nYAML内容: {}",
                recipe.name,
                req.description.unwrap_or_default(),
                req.content_yaml
            )),
        ));
    }

    Ok(Json(recipe_to_json(recipe)))
}

async fn delete_recipe(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let service = RecipeService::new((*state.db).clone());

    // Get recipe to find team_id
    let recipe = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Recipe not found".to_string()))?;

    // Check team permission
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&recipe.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can delete recipes
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can delete recipes".to_string(),
        ));
    }

    // Smart Log trigger (before delete)
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            recipe.team_id.to_hex(),
            user.0.clone(),
            "delete",
            "recipe",
            recipe.id.map(|id| id.to_hex()).unwrap_or_default(),
            recipe.name.clone(),
            None,
        ));
    }

    service
        .delete(&id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_recipe(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = RecipeService::new((*state.db).clone());
    let recipe = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Recipe not found".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&recipe.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can view recipes".to_string(),
        ));
    }

    Ok(Json(recipe_to_json(recipe)))
}

async fn update_recipe(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRecipeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let service = RecipeService::new((*state.db).clone());

    // Get recipe to find team_id
    let recipe = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Recipe not found".to_string()))?;

    // Check team permission
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&recipe.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    // Only admin/owner can update recipes
    if !can_manage_team(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team admin or owner can update recipes".to_string(),
        ));
    }

    let recipe = service
        .update(&id, req.name, req.description, req.content_yaml)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Smart Log trigger
    if let Some(trigger) = &state.smart_log_trigger {
        trigger.trigger(SmartLogContext::new(
            recipe.team_id.to_hex(),
            user.0.clone(),
            "update",
            "recipe",
            recipe.id.map(|id| id.to_hex()).unwrap_or_default(),
            recipe.name.clone(),
            Some(format!(
                "名称: {}\nYAML内容: {}",
                recipe.name, recipe.content_yaml
            )),
        ));
    }

    Ok(Json(recipe_to_json(recipe)))
}

/// Install a recipe (cloud server returns success)
async fn install_recipe(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUserId>,
    Path(id): Path<String>,
) -> Result<Json<InstallResponse>, (StatusCode, String)> {
    let service = RecipeService::new((*state.db).clone());

    // Verify recipe exists
    let recipe = service
        .get(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Recipe not found".to_string()))?;

    // Check team membership
    let team_service = TeamService::new((*state.db).clone());
    let team = team_service
        .get(&recipe.team_id.to_hex())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Team not found".to_string()))?;

    if !is_team_member(&team, &user.0) {
        return Err((
            StatusCode::FORBIDDEN,
            "Only team members can install recipes".to_string(),
        ));
    }

    // Increment use count
    let _ = service.increment_use_count(&id).await;

    Ok(Json(InstallResponse {
        success: true,
        resource_type: "recipe".to_string(),
        resource_id: recipe.id.map(|id| id.to_hex()).unwrap_or_default(),
        local_path: None,
        error: None,
    }))
}

/// Uninstall a recipe
async fn uninstall_recipe(Path(_id): Path<String>) -> Result<StatusCode, (StatusCode, String)> {
    // Cloud server just acknowledges the uninstall request
    // Actual uninstall happens on client side
    Ok(StatusCode::NO_CONTENT)
}
