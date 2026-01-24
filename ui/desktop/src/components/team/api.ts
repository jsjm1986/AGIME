// Team API functions

import {
  Team,
  TeamSummary,
  TeamMember,
  SharedSkill,
  SharedRecipe,
  SharedExtension,
  InstallResult,
  InstalledResource,
  PaginatedTeamsResponse,
  PaginatedMembersResponse,
  PaginatedSkillsResponse,
  PaginatedRecipesResponse,
  PaginatedExtensionsResponse,
  SkillStorageType,
  SkillFile,
  SkillManifest,
  SkillMetadata,
  ProtectionLevel,
  VerifyAccessRequest,
  VerifyAccessResponse,
  CreateInviteRequest,
  CreateInviteResponse,
  ValidateInviteResponse,
  AcceptInviteResponse,
  TeamInvite,
} from './types';
import { platform } from '../../platform';
import { getActiveServer } from './servers/serverStore';

const API_BASE = '/api/team';

// ============================================================
// Connection Mode Management
// ============================================================

// Connection mode type
export type TeamConnectionMode = 'lan' | 'cloud' | null;

// localStorage keys
const STORAGE_KEYS = {
  CONNECTION_MODE: 'AGIME_TEAM_CONNECTION_MODE',
  LAN_SERVER_URL: 'AGIME_TEAM_LAN_SERVER_URL',
  LAN_SECRET_KEY: 'AGIME_TEAM_LAN_SECRET_KEY',
  CLOUD_SERVER_URL: 'AGIME_TEAM_SERVER_URL',
  CLOUD_API_KEY: 'AGIME_TEAM_API_KEY',
};

// Get current connection mode
function getConnectionMode(): TeamConnectionMode {
  try {
    const mode = localStorage.getItem(STORAGE_KEYS.CONNECTION_MODE);
    if (mode === 'lan' || mode === 'cloud') return mode;
  } catch {
    // localStorage not available
  }
  return null;
}

// Set connection mode
export function setConnectionMode(mode: TeamConnectionMode): void {
  try {
    if (mode) {
      localStorage.setItem(STORAGE_KEYS.CONNECTION_MODE, mode);
    } else {
      localStorage.removeItem(STORAGE_KEYS.CONNECTION_MODE);
    }
  } catch {
    // localStorage not available
  }
}

// Get current connection mode (exported)
export function getTeamConnectionMode(): TeamConnectionMode {
  return getConnectionMode();
}

// Clear all remote connection settings
export function clearRemoteConnection(): void {
  try {
    localStorage.removeItem(STORAGE_KEYS.CONNECTION_MODE);
    localStorage.removeItem(STORAGE_KEYS.LAN_SERVER_URL);
    localStorage.removeItem(STORAGE_KEYS.LAN_SECRET_KEY);
    localStorage.removeItem(STORAGE_KEYS.CLOUD_SERVER_URL);
    localStorage.removeItem(STORAGE_KEYS.CLOUD_API_KEY);
  } catch {
    // localStorage not available
  }
}

// Get remote server configuration based on connection mode
function getRemoteServerConfig(): {
  url: string;
  authHeader: string;
  authValue: string;
} | null {
  const mode = getConnectionMode();

  try {
    // First, check for new multi-server system (cloud mode)
    if (mode === 'cloud') {
      const activeServer = getActiveServer();
      if (activeServer && activeServer.url && activeServer.apiKey) {
        return {
          url: activeServer.url.replace(/\/+$/, ''), // Remove trailing slash
          authHeader: 'X-API-Key',
          authValue: activeServer.apiKey,
        };
      }

      // Fall back to old single-server storage
      const url = localStorage.getItem(STORAGE_KEYS.CLOUD_SERVER_URL);
      const apiKey = localStorage.getItem(STORAGE_KEYS.CLOUD_API_KEY);
      if (url && url.trim() && apiKey) {
        return {
          url: url.trim(),
          authHeader: 'X-API-Key',
          authValue: apiKey,
        };
      }
    } else if (mode === 'lan') {
      const url = localStorage.getItem(STORAGE_KEYS.LAN_SERVER_URL);
      const secretKey = localStorage.getItem(STORAGE_KEYS.LAN_SECRET_KEY);
      if (url && url.trim() && secretKey) {
        return {
          url: url.trim(),
          authHeader: 'X-Secret-Key',
          authValue: secretKey,
        };
      }
    }
  } catch {
    // localStorage not available
  }
  return null;
}

// Helper function to get the full API URL
async function getApiUrl(path: string): Promise<string> {
  // Check for remote server configuration first
  const remoteConfig = getRemoteServerConfig();
  if (remoteConfig) {
    return `${remoteConfig.url}${API_BASE}${path}`;
  }

  // Fall back to local agimed server
  const hostPort = await platform.getAgimedHostPort();
  if (hostPort) {
    return `${hostPort}${API_BASE}${path}`;
  }
  // Fallback to relative URL (for web platform)
  return `${API_BASE}${path}`;
}

// Helper function for API calls with authentication
async function fetchApi<T>(
  path: string,
  options?: RequestInit
): Promise<T> {
  // Get remote config and URL
  const remoteConfig = getRemoteServerConfig();
  const url = await getApiUrl(path);

  // Build authentication headers
  const authHeaders: Record<string, string> = {};
  if (remoteConfig) {
    // Use remote server authentication
    authHeaders[remoteConfig.authHeader] = remoteConfig.authValue;
  } else {
    // Fall back to local agimed server
    const secretKey = await platform.getSecretKey();
    if (secretKey) {
      authHeaders['X-Secret-Key'] = secretKey;
    }
  }

  const response = await fetch(url, {
    ...options,
    headers: {
      'Content-Type': 'application/json',
      ...authHeaders,
      ...options?.headers,
    },
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }));
    throw new Error(error.message || `API Error: ${response.status}`);
  }

  return response.json();
}

// Helper function for binary API calls (file downloads)
async function fetchBinaryApi(
  path: string,
  options?: RequestInit
): Promise<Blob> {
  const remoteConfig = getRemoteServerConfig();
  const url = await getApiUrl(path);

  // Build authentication headers
  const authHeaders: Record<string, string> = {};
  if (remoteConfig) {
    authHeaders[remoteConfig.authHeader] = remoteConfig.authValue;
  } else {
    const secretKey = await platform.getSecretKey();
    if (secretKey) {
      authHeaders['X-Secret-Key'] = secretKey;
    }
  }

  const response = await fetch(url, {
    ...options,
    headers: {
      ...authHeaders,
      ...options?.headers,
    },
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }));
    throw new Error(error.message || `API Error: ${response.status}`);
  }

  return response.blob();
}

// Helper function for multipart form data uploads
async function fetchMultipartApi<T>(
  path: string,
  formData: FormData
): Promise<T> {
  const remoteConfig = getRemoteServerConfig();
  const url = await getApiUrl(path);

  // Build authentication headers
  const authHeaders: Record<string, string> = {};
  if (remoteConfig) {
    authHeaders[remoteConfig.authHeader] = remoteConfig.authValue;
  } else {
    const secretKey = await platform.getSecretKey();
    if (secretKey) {
      authHeaders['X-Secret-Key'] = secretKey;
    }
  }

  const response = await fetch(url, {
    method: 'POST',
    headers: {
      ...authHeaders,
      // Don't set Content-Type - browser will set it with boundary
    },
    body: formData,
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }));
    throw new Error(error.message || `API Error: ${response.status}`);
  }

  return response.json();
}

// Team API
export async function listTeams(page = 1, limit = 20): Promise<PaginatedTeamsResponse> {
  return fetchApi<PaginatedTeamsResponse>(`/teams?page=${page}&limit=${limit}`);
}

export async function getTeam(teamId: string): Promise<TeamSummary> {
  return fetchApi<TeamSummary>(`/teams/${teamId}`);
}

export async function createTeam(data: {
  name: string;
  description?: string;
  repositoryUrl?: string;
}): Promise<Team> {
  return fetchApi<Team>(`/teams`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function updateTeam(
  teamId: string,
  data: { name?: string; description?: string; repositoryUrl?: string }
): Promise<Team> {
  return fetchApi<Team>(`/teams/${teamId}`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
}

export async function deleteTeam(teamId: string): Promise<void> {
  await fetchApi<void>(`/teams/${teamId}`, { method: 'DELETE' });
}

// Members API
export async function listMembers(
  teamId: string,
  page = 1,
  limit = 50
): Promise<PaginatedMembersResponse> {
  return fetchApi<PaginatedMembersResponse>(
    `/teams/${teamId}/members?page=${page}&limit=${limit}`
  );
}

export async function addMember(
  teamId: string,
  data: { userId: string; displayName: string; role?: string; endpointUrl?: string }
): Promise<TeamMember> {
  return fetchApi<TeamMember>(`/teams/${teamId}/members`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function updateMember(
  memberId: string,
  data: { role?: string; displayName?: string }
): Promise<TeamMember> {
  return fetchApi<TeamMember>(`/members/${memberId}`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
}

export async function removeMember(memberId: string): Promise<void> {
  await fetchApi<void>(`/members/${memberId}`, { method: 'DELETE' });
}

export async function leaveTeam(teamId: string): Promise<void> {
  await fetchApi<void>(`/teams/${teamId}/leave`, { method: 'POST' });
}

// Invites API
export async function createInvite(
  teamId: string,
  data: CreateInviteRequest
): Promise<CreateInviteResponse> {
  return fetchApi<CreateInviteResponse>(`/teams/${teamId}/invites`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function validateInvite(code: string): Promise<ValidateInviteResponse> {
  return fetchApi<ValidateInviteResponse>(`/invites/${code}`);
}

export async function acceptInvite(
  code: string,
  displayName?: string
): Promise<AcceptInviteResponse> {
  return fetchApi<AcceptInviteResponse>(`/invites/${code}/accept`, {
    method: 'POST',
    body: JSON.stringify({ display_name: displayName }),
  });
}

export async function deleteInvite(teamId: string, code: string): Promise<void> {
  await fetchApi<void>(`/teams/${teamId}/invites/${code}`, { method: 'DELETE' });
}

export async function listInvites(teamId: string): Promise<{ invites: TeamInvite[]; total: number }> {
  return fetchApi<{ invites: TeamInvite[]; total: number }>(`/teams/${teamId}/invites`);
}

// Skills API
export async function listSkills(params: {
  teamId?: string;
  search?: string;
  tags?: string;
  page?: number;
  limit?: number;
}): Promise<PaginatedSkillsResponse> {
  const query = new URLSearchParams();
  if (params.teamId) query.set('teamId', params.teamId);
  if (params.search) query.set('search', params.search);
  if (params.tags) query.set('tags', params.tags);
  query.set('page', String(params.page || 1));
  query.set('limit', String(params.limit || 20));

  return fetchApi<PaginatedSkillsResponse>(`/skills?${query}`);
}

export async function shareSkill(data: {
  teamId: string;
  name: string;
  // Inline mode fields
  content?: string;
  // Package mode fields
  storageType?: SkillStorageType;
  skillMd?: string;
  files?: SkillFile[];
  manifest?: SkillManifest;
  metadata?: SkillMetadata;
  // Common fields
  description?: string;
  tags?: string[];
  visibility?: string;
  protectionLevel?: ProtectionLevel;
}): Promise<SharedSkill> {
  return fetchApi<SharedSkill>(`/skills`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function installSkill(skillId: string): Promise<InstallResult> {
  const mode = getConnectionMode();

  // If connected to cloud server, use two-step install
  if (mode === 'cloud') {
    // Step 1: Get skill content from cloud
    const skill = await fetchApi<SharedSkill>(`/skills/${skillId}`);

    // Step 2: Call local agimed to install
    const localUrl = await platform.getAgimedHostPort();
    if (!localUrl) {
      throw new Error('Local server not available');
    }

    const secretKey = await platform.getSecretKey();
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (secretKey) {
      headers['X-Secret-Key'] = secretKey;
    }

    const response = await fetch(`${localUrl}/api/team/skills/install-local`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        resourceId: skill.id,
        teamId: skill.teamId,
        name: skill.name,
        storageType: skill.storageType || 'inline',
        content: skill.content,
        skillMd: skill.skillMd,
        files: skill.files,
        version: skill.version,
        protectionLevel: skill.protectionLevel,
      }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: response.statusText }));
      throw new Error(error.message || `Install failed: ${response.status}`);
    }

    return response.json();
  }

  // Local server: use original install API
  return fetchApi<InstallResult>(`/skills/${skillId}/install`, {
    method: 'POST',
  });
}

export async function uninstallSkill(skillId: string): Promise<void> {
  await fetchApi<void>(`/skills/${skillId}/uninstall`, { method: 'DELETE' });
}

export async function getSkill(skillId: string): Promise<SharedSkill> {
  return fetchApi<SharedSkill>(`/skills/${skillId}`);
}

export async function updateSkill(
  skillId: string,
  data: {
    name?: string;
    // Inline mode fields
    content?: string;
    // Package mode fields
    storageType?: SkillStorageType;
    skillMd?: string;
    files?: SkillFile[];
    manifest?: SkillManifest;
    metadata?: SkillMetadata;
    // Common fields
    description?: string;
    tags?: string[];
    visibility?: string;
  }
): Promise<SharedSkill> {
  return fetchApi<SharedSkill>(`/skills/${skillId}`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
}

export async function deleteSkill(skillId: string): Promise<void> {
  await fetchApi<void>(`/skills/${skillId}`, { method: 'DELETE' });
}

// ============================================================
// Local Skills API - List installed local skills for sharing
// ============================================================

/**
 * Local skill info for sharing
 */
export interface LocalSkill {
  name: string;
  description: string;
  path: string;
  storageType: 'inline' | 'package';
  content?: string;
  skillMd?: string;
  files?: SkillFile[];
}

/**
 * List local skills installed on this machine
 * This calls the local agimed server to scan skill directories
 */
export async function listLocalSkills(): Promise<LocalSkill[]> {
  // Always call local server for local skills
  const localUrl = await platform.getAgimedHostPort();
  if (!localUrl) {
    throw new Error('Local server not available');
  }

  const secretKey = await platform.getSecretKey();
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  if (secretKey) {
    headers['X-Secret-Key'] = secretKey;
  }

  const response = await fetch(`${localUrl}/api/team/skills/local`, {
    method: 'GET',
    headers,
  });

  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }));
    throw new Error(error.message || `API Error: ${response.status}`);
  }

  const data = await response.json();
  return data.skills || [];
}

// ============================================================
// Skill Package API - Agent Skills Open Standard Support
// See: https://agentskills.io/specification
// ============================================================

/**
 * Upload a ZIP package to create a new skill
 * The ZIP must contain a SKILL.md file with YAML frontmatter
 */
export async function uploadSkillPackage(
  teamId: string,
  file: File,
  options?: {
    visibility?: string;
    tags?: string[];
    protectionLevel?: ProtectionLevel;
  }
): Promise<SharedSkill> {
  const formData = new FormData();
  formData.append('file', file);
  formData.append('teamId', teamId);
  if (options?.visibility) {
    formData.append('visibility', options.visibility);
  }
  if (options?.tags && options.tags.length > 0) {
    formData.append('tags', JSON.stringify(options.tags));
  }
  if (options?.protectionLevel) {
    formData.append('protectionLevel', options.protectionLevel);
  }

  return fetchMultipartApi<SharedSkill>(`/skills/import`, formData);
}

/**
 * Download a skill as a ZIP package
 * Returns the skill packaged with SKILL.md and all attached files
 */
export async function downloadSkillPackage(skillId: string): Promise<Blob> {
  return fetchBinaryApi(`/skills/${skillId}/export`);
}

/**
 * Export skill as ZIP (alias for downloadSkillPackage)
 */
export const exportSkill = downloadSkillPackage;

/**
 * List all files in a skill package
 */
export async function listSkillFiles(skillId: string): Promise<SkillFile[]> {
  const response = await fetchApi<{ files: SkillFile[] }>(`/skills/${skillId}/files`);
  return response.files;
}

/**
 * Get a single file from a skill package
 * @param skillId - Skill ID
 * @param filePath - Relative path within the package (e.g., "scripts/lint.py")
 * @returns File content as string (text) or base64 (binary)
 */
export async function getSkillFile(
  skillId: string,
  filePath: string
): Promise<{ content: string; contentType: string; isBinary: boolean }> {
  return fetchApi<{ content: string; contentType: string; isBinary: boolean }>(
    `/skills/${skillId}/files/${encodeURIComponent(filePath)}`
  );
}

/**
 * Add or update a file in a skill package
 */
export async function addSkillFile(
  skillId: string,
  file: {
    path: string;
    content: string;
    contentType?: string;
    isBinary?: boolean;
  }
): Promise<SkillFile> {
  return fetchApi<SkillFile>(`/skills/${skillId}/files`, {
    method: 'POST',
    body: JSON.stringify(file),
  });
}

/**
 * Delete a file from a skill package
 */
export async function deleteSkillFile(
  skillId: string,
  filePath: string
): Promise<void> {
  await fetchApi<void>(`/skills/${skillId}/files/${encodeURIComponent(filePath)}`, {
    method: 'DELETE',
  });
}

/**
 * Convert an inline skill to package format
 * Wraps the existing content in a SKILL.md with proper frontmatter
 */
export async function convertToPackageSkill(
  skillId: string
): Promise<SharedSkill> {
  return fetchApi<SharedSkill>(`/skills/${skillId}/convert-to-package`, {
    method: 'POST',
  });
}

/**
 * Validate a skill package (check SKILL.md format, file paths, etc.)
 */
export async function validateSkillPackage(
  file: File
): Promise<{
  valid: boolean;
  errors: string[];
  warnings: string[];
  parsed?: {
    name: string;
    description: string;
    fileCount: number;
    totalSize: number;
  };
}> {
  const formData = new FormData();
  formData.append('file', file);

  return fetchMultipartApi<{
    valid: boolean;
    errors: string[];
    warnings: string[];
    parsed?: {
      name: string;
      description: string;
      fileCount: number;
      totalSize: number;
    };
  }>(`/skills/validate-package`, formData);
}

// Recipes API
export async function listRecipes(params: {
  teamId?: string;
  search?: string;
  category?: string;
  tags?: string;
  page?: number;
  limit?: number;
}): Promise<PaginatedRecipesResponse> {
  const query = new URLSearchParams();
  if (params.teamId) query.set('teamId', params.teamId);
  if (params.search) query.set('search', params.search);
  if (params.category) query.set('category', params.category);
  if (params.tags) query.set('tags', params.tags);
  query.set('page', String(params.page || 1));
  query.set('limit', String(params.limit || 20));

  return fetchApi<PaginatedRecipesResponse>(`/recipes?${query}`);
}

export async function shareRecipe(data: {
  teamId: string;
  name: string;
  contentYaml: string;
  description?: string;
  category?: string;
  tags?: string[];
  visibility?: string;
  protectionLevel?: ProtectionLevel;
}): Promise<SharedRecipe> {
  return fetchApi<SharedRecipe>(`/recipes`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function installRecipe(recipeId: string): Promise<InstallResult> {
  const mode = getConnectionMode();

  // If connected to cloud server, use two-step install
  if (mode === 'cloud') {
    // Step 1: Get recipe content from cloud
    const recipe = await fetchApi<SharedRecipe>(`/recipes/${recipeId}`);

    // Step 2: Call local agimed to install
    const localUrl = await platform.getAgimedHostPort();
    if (!localUrl) {
      throw new Error('Local server not available');
    }

    const secretKey = await platform.getSecretKey();
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (secretKey) {
      headers['X-Secret-Key'] = secretKey;
    }

    const response = await fetch(`${localUrl}/api/team/recipes/install-local`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        resourceId: recipe.id,
        teamId: recipe.teamId,
        name: recipe.name,
        contentYaml: recipe.contentYaml,
        version: recipe.version,
        protectionLevel: recipe.protectionLevel,
      }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: response.statusText }));
      throw new Error(error.message || `Install failed: ${response.status}`);
    }

    return response.json();
  }

  // Local server: use original install API
  return fetchApi<InstallResult>(`/recipes/${recipeId}/install`, {
    method: 'POST',
  });
}

export async function uninstallRecipe(recipeId: string): Promise<void> {
  await fetchApi<void>(`/recipes/${recipeId}/uninstall`, { method: 'DELETE' });
}

export async function getRecipe(recipeId: string): Promise<SharedRecipe> {
  return fetchApi<SharedRecipe>(`/recipes/${recipeId}`);
}

export async function updateRecipe(
  recipeId: string,
  data: {
    name?: string;
    contentYaml?: string;
    description?: string;
    category?: string;
    tags?: string[];
    visibility?: string;
    protectionLevel?: ProtectionLevel;
  }
): Promise<SharedRecipe> {
  return fetchApi<SharedRecipe>(`/recipes/${recipeId}`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
}

export async function deleteRecipe(recipeId: string): Promise<void> {
  await fetchApi<void>(`/recipes/${recipeId}`, { method: 'DELETE' });
}

// Extensions API
export async function listExtensions(params: {
  teamId?: string;
  search?: string;
  extensionType?: string;
  tags?: string;
  reviewedOnly?: boolean;
  page?: number;
  limit?: number;
}): Promise<PaginatedExtensionsResponse> {
  const query = new URLSearchParams();
  if (params.teamId) query.set('teamId', params.teamId);
  if (params.search) query.set('search', params.search);
  if (params.extensionType) query.set('extensionType', params.extensionType);
  if (params.tags) query.set('tags', params.tags);
  if (params.reviewedOnly !== undefined) query.set('reviewedOnly', String(params.reviewedOnly));
  query.set('page', String(params.page || 1));
  query.set('limit', String(params.limit || 20));

  return fetchApi<PaginatedExtensionsResponse>(`/extensions?${query}`);
}

export async function shareExtension(data: {
  teamId: string;
  name: string;
  extensionType: string;
  config: Record<string, unknown>;
  description?: string;
  tags?: string[];
  visibility?: string;
  protectionLevel?: ProtectionLevel;
}): Promise<SharedExtension> {
  return fetchApi<SharedExtension>(`/extensions`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

export async function installExtension(extensionId: string): Promise<InstallResult> {
  const mode = getConnectionMode();

  // If connected to cloud server, use two-step install
  if (mode === 'cloud') {
    // Step 1: Get extension content from cloud
    const extension = await fetchApi<SharedExtension>(`/extensions/${extensionId}`);

    // Step 2: Call local agimed to install
    const localUrl = await platform.getAgimedHostPort();
    if (!localUrl) {
      throw new Error('Local server not available');
    }

    const secretKey = await platform.getSecretKey();
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    if (secretKey) {
      headers['X-Secret-Key'] = secretKey;
    }

    const response = await fetch(`${localUrl}/api/team/extensions/install-local`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        resourceId: extension.id,
        teamId: extension.teamId,
        name: extension.name,
        extensionType: extension.extensionType,
        config: extension.config,
        version: extension.version,
        protectionLevel: extension.protectionLevel,
      }),
    });

    if (!response.ok) {
      const error = await response.json().catch(() => ({ message: response.statusText }));
      throw new Error(error.message || `Install failed: ${response.status}`);
    }

    return response.json();
  }

  // Local server: use original install API
  return fetchApi<InstallResult>(`/extensions/${extensionId}/install`, {
    method: 'POST',
  });
}

export async function uninstallExtension(extensionId: string): Promise<void> {
  await fetchApi<void>(`/extensions/${extensionId}/uninstall`, { method: 'DELETE' });
}

export async function getExtension(extensionId: string): Promise<SharedExtension> {
  return fetchApi<SharedExtension>(`/extensions/${extensionId}`);
}

export async function updateExtension(
  extensionId: string,
  data: {
    name?: string;
    description?: string;
    config?: Record<string, unknown>;
    tags?: string[];
    visibility?: string;
    protectionLevel?: ProtectionLevel;
  }
): Promise<SharedExtension> {
  return fetchApi<SharedExtension>(`/extensions/${extensionId}`, {
    method: 'PUT',
    body: JSON.stringify(data),
  });
}

export async function deleteExtension(extensionId: string): Promise<void> {
  await fetchApi<void>(`/extensions/${extensionId}`, { method: 'DELETE' });
}

export async function reviewExtension(
  extensionId: string,
  data: { approved: boolean; notes?: string }
): Promise<SharedExtension> {
  return fetchApi<SharedExtension>(`/extensions/${extensionId}/review`, {
    method: 'POST',
    body: JSON.stringify(data),
  });
}

// Sync/Installed resources API
export async function listInstalled(): Promise<InstalledResource[]> {
  const response = await fetchApi<{ resources: InstalledResource[] }>(
    `/resources/installed`
  );
  return response.resources;
}

export async function checkUpdates(resourceIds: string[]): Promise<
  Array<{
    resourceId: string;
    resourceType: string;
    resourceName: string;
    currentVersion: string;
    latestVersion: string;
    hasUpdate: boolean;
  }>
> {
  const response = await fetchApi<{
    updates: Array<{
      resourceId: string;
      resourceType: string;
      resourceName: string;
      currentVersion: string;
      latestVersion: string;
      hasUpdate: boolean;
    }>;
  }>(`/resources/check-updates`, {
    method: 'POST',
    body: JSON.stringify({ resourceIds }),
  });
  return response.updates;
}

// Recommendations API
export interface Recommendation {
  resourceId: string;
  resourceType: 'skill' | 'recipe' | 'extension';
  resourceName: string;
  teamId: string;
  description: string | null;
  score: number;
  reason: 'popular' | 'personal_history' | 'similar_content' | 'trending' | 'new' | 'collaborative_filtering';
  tags: string[];
}

export async function getRecommendations(params: {
  teamId?: string;
  userId?: string;
  context?: string;
  limit?: number;
}): Promise<Recommendation[]> {
  const query = new URLSearchParams();
  if (params.teamId) query.set('teamId', params.teamId);
  if (params.userId) query.set('userId', params.userId);
  if (params.context) query.set('context', params.context);
  query.set('limit', String(params.limit || 10));

  return fetchApi<Recommendation[]>(`/recommendations?${query}`);
}

// Batch install API
export async function batchInstall(resourceIds: string[]): Promise<{
  results: Array<{
    resourceId: string;
    success: boolean;
    error?: string;
  }>;
}> {
  return fetchApi<{
    results: Array<{
      resourceId: string;
      success: boolean;
      error?: string;
    }>;
  }>(`/resources/batch-install`, {
    method: 'POST',
    body: JSON.stringify({ resourceIds }),
  });
}

// Verify access API - types imported from ./types

export async function verifySkillAccess(
  skillId: string,
  request?: VerifyAccessRequest
): Promise<VerifyAccessResponse> {
  return fetchApi<VerifyAccessResponse>(`/skills/${skillId}/verify-access`, {
    method: 'POST',
    body: JSON.stringify(request || {}),
  });
}

// Get cleanup count before removing member
export interface CleanupCountResponse {
  count: number;
}

export async function getCleanupCount(
  teamId: string,
  userId: string
): Promise<CleanupCountResponse> {
  return fetchApi<CleanupCountResponse>(
    `/teams/${teamId}/members/cleanup-count?userId=${encodeURIComponent(userId)}`
  );
}

// Remove member with cleanup result
export interface RemoveMemberResult {
  memberId: string;
  teamId: string;
  userId: string;
  cleanedCount: number;
  failures: number;
}

export async function removeMemberWithCleanup(memberId: string): Promise<RemoveMemberResult> {
  return fetchApi<RemoveMemberResult>(`/members/${memberId}`, { method: 'DELETE' });
}

// Sync API
export interface SyncStatus {
  teamId: string;
  state: string; // 'idle' | 'syncing' | 'error' | 'up_to_date' | 'needs_pull'
  lastSyncAt: string | null;
  lastCommitHash: string | null;
  errorMessage: string | null;
}

export async function getSyncStatus(teamId: string): Promise<SyncStatus> {
  return fetchApi<SyncStatus>(`/teams/${teamId}/sync/status`);
}

export async function triggerSync(teamId: string): Promise<{ message: string }> {
  return fetchApi<{ message: string }>(`/teams/${teamId}/sync`, {
    method: 'POST',
  });
}

// Service health check
export interface ServiceHealth {
  online: boolean;
  latency?: number;
  error?: string;
}

export async function checkServiceHealth(): Promise<ServiceHealth> {
  const startTime = Date.now();
  try {
    const remoteConfig = getRemoteServerConfig();
    const url = await getApiUrl('/teams?page=1&limit=1');

    // Build authentication headers
    const authHeaders: Record<string, string> = {};
    if (remoteConfig) {
      authHeaders[remoteConfig.authHeader] = remoteConfig.authValue;
    } else {
      const secretKey = await platform.getSecretKey();
      if (secretKey) {
        authHeaders['X-Secret-Key'] = secretKey;
      }
    }

    const response = await fetch(url, {
      method: 'GET',
      headers: {
        'Content-Type': 'application/json',
        ...authHeaders,
      },
      signal: AbortSignal.timeout(5000), // 5 second timeout
    });

    const latency = Date.now() - startTime;

    if (response.ok) {
      return { online: true, latency };
    } else {
      return { online: false, latency, error: `HTTP ${response.status}` };
    }
  } catch (error) {
    return {
      online: false,
      error: error instanceof Error ? error.message : 'Connection failed'
    };
  }
}
