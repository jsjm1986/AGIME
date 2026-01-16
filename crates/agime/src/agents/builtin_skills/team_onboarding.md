# Team Collaboration Onboarding Guide

This guide helps you get started with AGIME's team collaboration features.

## Quick Start

### 1. Check Your Teams

First, see which teams you belong to:

```
Use tool: team_list
Parameters: { "include_stats": true }
```

This will show all teams you're a member of, along with resource statistics.

### 2. Discover Team Resources

Search for resources shared by your team:

```
# Search for Skills
Use tool: team_search
Parameters: { "resource_type": "skills", "limit": 10 }

# Search for Recipes (workflow automations)
Use tool: team_search
Parameters: { "resource_type": "recipes", "limit": 10 }

# Search for Extensions (MCP integrations)
Use tool: team_search
Parameters: { "resource_type": "extensions", "limit": 10 }
```

### 3. Get Recommendations

Get personalized recommendations based on your team's activity:

```
Use tool: team_get_recommendations
Parameters: { "limit": 5 }
```

## Installing Resources

### Install a Skill Locally

When you find a useful skill, install it for offline use:

```
Use tool: team_install
Parameters: { "resource_type": "skill", "resource_id": "<skill-id-from-search>" }
```

The skill will be saved to `~/.agime/skills/<skill-name>/` and can be loaded with `loadSkill`.

### Protection Levels

Resources have different protection levels:

| Level | Local Install | Offline Use | Notes |
|-------|--------------|-------------|-------|
| `public` | Yes | Yes | No restrictions |
| `team_installable` | Yes | 72h grace | Default for team resources |
| `team_online_only` | No | No | Use `team_load_skill` instead |
| `controlled` | No | No | Requires special approval |

## Sharing Resources

### Share a Skill

Share your knowledge with the team:

```
Use tool: team_share_skill
Parameters: {
  "team_id": "<your-team-id>",
  "name": "my-awesome-skill",
  "content": "# Skill Content\n\nYour markdown instructions here...",
  "description": "What this skill does",
  "tags": ["category", "topic"]
}
```

### Share a Recipe

Share workflow automations:

```
Use tool: team_share_recipe
Parameters: {
  "team_id": "<your-team-id>",
  "name": "daily-report-automation",
  "content_yaml": "steps:\n  - action: collect_data\n  - action: generate_report",
  "description": "Automates daily reporting",
  "category": "automation",
  "tags": ["daily", "reporting"]
}
```

### Share an Extension

Share MCP extension configurations (requires security review):

```
Use tool: team_share_extension
Parameters: {
  "team_id": "<your-team-id>",
  "name": "github-integration",
  "extension_type": "stdio",
  "config": "{\"command\": \"npx\", \"args\": [\"-y\", \"@modelcontextprotocol/server-github\"]}",
  "description": "GitHub MCP server integration",
  "tags": ["github", "integration"]
}
```

## Managing Installed Resources

### List Installed Resources

See what you have installed locally:

```
Use tool: team_list_installed
Parameters: { "resource_type": "skill" }
```

### Check for Updates

Check if any installed resources have updates:

```
Use tool: team_check_updates
Parameters: {}
```

### Uninstall a Resource

Remove a locally installed resource:

```
Use tool: team_uninstall_local
Parameters: { "resource_type": "skill", "resource_id": "<resource-id>" }
```

## Best Practices

1. **Search before creating** - Check if a similar resource exists
2. **Use descriptive tags** - Helps others discover your resources
3. **Keep skills focused** - One skill, one purpose
4. **Document thoroughly** - Include examples and use cases
5. **Check updates regularly** - Stay current with team improvements

## Workflow Example: New Team Member

```
1. team_list → See your teams
2. team_get_recommendations → Discover popular resources
3. team_search("skills", query="onboarding") → Find onboarding materials
4. team_install("skill", "<id>") → Install useful skills
5. loadSkill("installed-skill-name") → Use the skill
```

## Troubleshooting

### "Authorization expired"
- Reinstall the skill: `team_install`
- Or use online loading: `team_load_skill`

### "Skill not found"
- Check the skill name with `team_search`
- Ensure you have team membership

### "Protection level does not allow local install"
- Use `team_load_skill` for online-only resources
- Contact team admin for controlled resources
