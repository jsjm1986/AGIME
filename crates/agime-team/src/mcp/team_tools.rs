//! Team MCP tools definitions

use serde::{Deserialize, Serialize};

/// Team tool enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamTool {
    /// Search team skills
    SearchSkills,
    /// Search team recipes
    SearchRecipes,
    /// Search team extensions
    SearchExtensions,
    /// Load a skill into context
    LoadSkill,
    /// Run a recipe
    RunRecipe,
    /// Share a skill
    ShareSkill,
    /// Share a recipe
    ShareRecipe,
    /// Get AI recommendations
    RecommendResources,
    /// List installed resources
    ListInstalledResources,
    /// Check for updates
    CheckUpdates,
}

impl TeamTool {
    /// Get tool name
    pub fn name(&self) -> &'static str {
        match self {
            TeamTool::SearchSkills => "team_search_skills",
            TeamTool::SearchRecipes => "team_search_recipes",
            TeamTool::SearchExtensions => "team_search_extensions",
            TeamTool::LoadSkill => "team_load_skill",
            TeamTool::RunRecipe => "team_run_recipe",
            TeamTool::ShareSkill => "team_share_skill",
            TeamTool::ShareRecipe => "team_share_recipe",
            TeamTool::RecommendResources => "team_recommend",
            TeamTool::ListInstalledResources => "team_list_installed",
            TeamTool::CheckUpdates => "team_check_updates",
        }
    }

    /// Get tool description
    pub fn description(&self) -> &'static str {
        match self {
            TeamTool::SearchSkills => "Search for skills shared by your team",
            TeamTool::SearchRecipes => "Search for recipes shared by your team",
            TeamTool::SearchExtensions => "Search for extensions shared by your team",
            TeamTool::LoadSkill => "Load a team skill into the current context",
            TeamTool::RunRecipe => "Execute a team recipe",
            TeamTool::ShareSkill => "Share the current skill with your team",
            TeamTool::ShareRecipe => "Share the current recipe with your team",
            TeamTool::RecommendResources => "Get AI-powered recommendations for team resources",
            TeamTool::ListInstalledResources => "List all installed team resources",
            TeamTool::CheckUpdates => "Check for updates to installed resources",
        }
    }

    /// Get all tools
    pub fn all() -> Vec<TeamTool> {
        vec![
            TeamTool::SearchSkills,
            TeamTool::SearchRecipes,
            TeamTool::SearchExtensions,
            TeamTool::LoadSkill,
            TeamTool::RunRecipe,
            TeamTool::ShareSkill,
            TeamTool::ShareRecipe,
            TeamTool::RecommendResources,
            TeamTool::ListInstalledResources,
            TeamTool::CheckUpdates,
        ]
    }
}

/// Search parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchParams {
    pub query: Option<String>,
    pub team_id: Option<String>,
    pub tags: Option<Vec<String>>,
    pub limit: Option<u32>,
}

/// Load skill parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadSkillParams {
    pub skill_id: String,
}

/// Share skill parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareSkillParams {
    pub team_id: String,
    pub name: String,
    pub content: String,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
}
