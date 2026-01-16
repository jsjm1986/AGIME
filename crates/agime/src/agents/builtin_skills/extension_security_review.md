# MCP Extension Security Review Guide

This guide provides a framework for evaluating and reviewing MCP extensions shared within your team.

## Overview

MCP (Model Context Protocol) extensions can execute code on your system. Before using or approving team-shared extensions, conduct a thorough security review.

## Security Review Checklist

### 1. Extension Type Analysis

| Type | Risk Level | Concerns |
|------|------------|----------|
| `stdio` | High | Executes local commands |
| `sse` | Medium | Network connections |

**Questions to ask:**
- What command does this extension execute?
- Does it require network access?
- What file system access does it need?

### 2. Command Analysis (for stdio extensions)

Examine the extension configuration:

```json
{
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-github"],
  "env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" }
}
```

**Check for:**

#### 2.1 Command Source
- [ ] Is the command from a trusted source?
- [ ] Is the package from npm/official registries?
- [ ] Can you verify the package maintainer?

#### 2.2 Arguments Analysis
- [ ] Are all arguments necessary?
- [ ] Any suspicious flags (e.g., `--allow-net`, `--allow-all`)?
- [ ] Any hardcoded paths that could be malicious?

#### 2.3 Environment Variables
- [ ] What secrets are required?
- [ ] Are secrets properly scoped?
- [ ] No hardcoded credentials?

### 3. Network Security

For SSE-type extensions or extensions that make network calls:

```
Check the endpoint URL:
- Is it HTTPS?
- Is it an internal/trusted domain?
- Does it require authentication?
```

**Red flags:**
- HTTP (not HTTPS) endpoints
- Unknown/untrusted domains
- Endpoints in suspicious regions
- No authentication required for sensitive operations

### 4. Permission Scope

Evaluate what the extension can access:

| Permission | Risk | Questions |
|------------|------|-----------|
| File System | High | What directories? Read/Write? |
| Network | Medium | Which endpoints? What data? |
| Environment | Medium | Which variables? Sensitive data? |
| Subprocess | High | What commands? With what privileges? |

### 5. Code Review (if source available)

If you can access the extension source:

```
1. Check package.json dependencies
2. Review main entry point
3. Look for:
   - eval() or Function() calls
   - Dynamic imports
   - Shell command execution
   - File system operations
   - Network requests
   - Credential handling
```

## Risk Assessment Matrix

| Factor | Low | Medium | High | Critical |
|--------|-----|--------|------|----------|
| Source | Official/Verified | Known maintainer | Unknown | Suspicious |
| Permissions | Read-only | Limited write | Full access | Admin/Root |
| Network | None | Internal only | External API | Unrestricted |
| Data access | Public | Team data | User data | Credentials |

## Review Process

### Step 1: Gather Information

```
Use tool: team_search
Parameters: { "resource_type": "extensions", "query": "<extension-name>" }
```

Get the extension details including:
- Who shared it
- When it was shared
- Tags and description
- Configuration

### Step 2: Document Findings

Create a security review document:

```markdown
## Extension: [Name]
**Reviewer:** [Your name]
**Date:** [Date]
**Version:** [Version]

### Summary
[Brief description of what the extension does]

### Risk Assessment
- Overall Risk: [Low/Medium/High/Critical]
- Recommended Action: [Approve/Reject/Needs Changes]

### Findings
1. [Finding 1]
2. [Finding 2]

### Recommendations
1. [Recommendation 1]
2. [Recommendation 2]
```

### Step 3: Approval Decision

| Decision | Criteria |
|----------|----------|
| **Approve** | Low risk, verified source, necessary functionality |
| **Approve with conditions** | Medium risk, needs monitoring/limitations |
| **Request changes** | Fixable security concerns |
| **Reject** | High/Critical risk, unverifiable source |

## Common Vulnerabilities to Check

### 1. Command Injection
```json
// DANGEROUS - User input in command
{ "command": "sh", "args": ["-c", "${USER_INPUT}"] }

// SAFE - Fixed command with validated input
{ "command": "git", "args": ["log", "--oneline", "-n", "10"] }
```

### 2. Path Traversal
```json
// DANGEROUS - Unrestricted path
{ "args": ["--config", "${CONFIG_PATH}"] }

// SAFE - Validated path within allowed directory
{ "args": ["--config", "/app/config/settings.json"] }
```

### 3. Credential Exposure
```json
// DANGEROUS - Token in args (visible in process list)
{ "args": ["--token", "ghp_xxxxxxxxxxxx"] }

// SAFE - Token in environment variable
{ "env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" } }
```

### 4. Privilege Escalation
```json
// DANGEROUS - Running as root/admin
{ "command": "sudo", "args": ["..."] }

// SAFE - Minimal required privileges
{ "command": "node", "args": ["server.js"] }
```

## Team Extension Sharing Guidelines

When sharing extensions with your team:

1. **Document thoroughly** - Explain what the extension does and why
2. **Minimal permissions** - Request only necessary permissions
3. **Verified sources** - Use official packages when possible
4. **Version pinning** - Specify exact versions, not latest
5. **Security notes** - Include any security considerations

### Example of Well-Documented Extension Share

```
Use tool: team_share_extension
Parameters: {
  "team_id": "<team-id>",
  "name": "github-readonly",
  "extension_type": "stdio",
  "config": "{
    \"command\": \"npx\",
    \"args\": [\"-y\", \"@modelcontextprotocol/server-github@1.2.3\"],
    \"env\": {\"GITHUB_TOKEN\": \"${GITHUB_TOKEN}\"}
  }",
  "description": "Read-only GitHub integration for viewing repos and issues. Requires GITHUB_TOKEN with 'repo:read' scope only. Verified package from MCP official.",
  "tags": ["github", "readonly", "verified", "low-risk"]
}
```

## Emergency Response

If you discover a malicious extension:

1. **Immediately notify** team admins
2. **Do not use** the extension
3. **Check usage logs** - Has anyone installed it?
4. **Revoke access** - Remove from team resources
5. **Audit systems** - Check for compromise indicators

## Resources

- MCP Official Documentation: https://modelcontextprotocol.io
- npm Package Security: https://www.npmjs.com/advisories
- OWASP Guidelines: https://owasp.org/www-project-top-ten/
