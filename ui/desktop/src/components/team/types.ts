// Team feature types

// ============================================================
// Cloud Server Types (for multi-server support)
// ============================================================

/** Cloud server connection status */
export type ServerConnectionStatus = 'online' | 'offline' | 'connecting' | 'error';

/** Cloud server configuration */
export interface CloudServer {
  /** Unique identifier */
  id: string;
  /** User-defined server name */
  name: string;
  /** Server URL (e.g., https://team.company.com) */
  url: string;
  /** API Key for authentication */
  apiKey: string;
  /** User email on this server */
  userEmail?: string;
  /** User display name on this server */
  displayName?: string;
  /** User ID on this server */
  userId?: string;
  /** Number of teams on this server */
  teamsCount: number;
  /** Connection status */
  status: ServerConnectionStatus;
  /** Last successful sync time */
  lastSyncedAt?: string;
  /** Last error message */
  lastError?: string;
  /** Creation time */
  createdAt: string;
}

/** Create cloud server request */
export interface CreateCloudServerRequest {
  name: string;
  url: string;
  apiKey: string;
}

/** Server health check response */
export interface ServerHealthResponse {
  status: 'healthy' | 'unhealthy';
  database?: string;
  version?: string;
  error?: string;
}

// ============================================================
// Team Invite Types
// ============================================================

/** Invite expiration options */
export type InviteExpiration = '24h' | '7d' | '30d' | 'never';

/** Invite role */
export type InviteRole = 'member' | 'admin';

/** Team invite */
export interface TeamInvite {
  /** Invite code */
  code: string;
  /** Full invite URL */
  url: string;
  /** Team ID */
  teamId: string;
  /** Role assigned to invitee */
  role: InviteRole;
  /** Expiration time (ISO 8601) */
  expiresAt?: string;
  /** Maximum uses (null = unlimited) */
  maxUses?: number;
  /** Current use count */
  usedCount: number;
  /** Creator user ID */
  createdBy: string;
  /** Creation time */
  createdAt: string;
}

/** Create invite request - uses snake_case to match backend */
export interface CreateInviteRequest {
  /** Expiration duration */
  expires_in: InviteExpiration;
  /** Maximum uses (null = unlimited) */
  max_uses?: number;
  /** Role for invitee */
  role: InviteRole;
}

/** Create invite response */
export interface CreateInviteResponse {
  code: string;
  url: string;
  expiresAt?: string;
  maxUses?: number;
  usedCount: number;
}

/** Validate invite response */
export interface ValidateInviteResponse {
  valid: boolean;
  teamId?: string;
  teamName?: string;
  teamDescription?: string;
  role?: InviteRole;
  inviterName?: string;
  expiresAt?: string;
  error?: string;
}

/** Accept invite response */
export interface AcceptInviteResponse {
  success: boolean;
  teamId?: string;
  memberId?: string;
  error?: string;
}

// ============================================================
// LAN Types (for local network peer connections)
// ============================================================

/** LAN connection status */
export type LANConnectionStatus = 'connected' | 'disconnected' | 'connecting' | 'error';

/** Discovered LAN device (via mDNS or manual) */
export interface DiscoveredDevice {
  /** Device/host name */
  name: string;
  /** IP address or hostname */
  host: string;
  /** Port number */
  port: number;
  /** AGIME version */
  version?: string;
  /** Number of teams available */
  teamsCount?: number;
  /** Whether currently reachable */
  isOnline: boolean;
  /** Last seen timestamp */
  lastSeen?: string;
}

/** Saved LAN connection */
export interface LANConnection {
  /** Unique identifier */
  id: string;
  /** User-defined name */
  name: string;
  /** Host address (IP or hostname) */
  host: string;
  /** Port number */
  port: number;
  /** Secret key for authentication (encrypted) */
  secretKey: string;
  /** Display name shown to remote user */
  myNickname: string;
  /** Connection status */
  status: LANConnectionStatus;
  /** Last online timestamp */
  lastOnline?: string;
  /** Last error message */
  lastError?: string;
  /** Teams available on this connection */
  teamsCount?: number;
  /** Creation time */
  createdAt: string;
}

/** Request to add LAN connection */
export interface AddLANConnectionRequest {
  name: string;
  host: string;
  port: number;
  secretKey: string;
  myNickname: string;
}

/** LAN sharing settings */
export interface LANShareSettings {
  /** Whether LAN sharing is enabled */
  enabled: boolean;
  /** The secret key to share with others */
  secretKey?: string;
  /** Port number for LAN connections */
  port: number;
  /** Display name shown to others */
  displayName: string;
}

/** LAN scan result */
export interface LANScanResult {
  devices: DiscoveredDevice[];
  scanDuration: number;
}

// ============================================================
// Protection Level (for sensitive content control)
// ============================================================

/** Protection level for shared resources */
export type ProtectionLevel = 'public' | 'team_installable' | 'team_online_only' | 'controlled';

/** Check if a protection level allows local installation */
export const allowsLocalInstall = (level: ProtectionLevel): boolean =>
  level === 'public' || level === 'team_installable';

/** Check if a protection level requires authorization */
export const requiresAuthorization = (level: ProtectionLevel): boolean =>
  level !== 'public';

// ============================================================
// Authorization Types
// ============================================================

/** Authorization info for installed resources */
export interface Authorization {
  /** Authorization token */
  token: string;
  /** Token expiration time (ISO 8601) */
  expiresAt: string;
  /** Last verification time (ISO 8601) */
  lastVerifiedAt: string;
}

/** Authorization status */
export type AuthorizationStatus = 'not_required' | 'valid' | 'needs_refresh' | 'expired' | 'missing';

/** Verify access request */
export interface VerifyAccessRequest {
  userId?: string;
}

/** Verify access response */
export interface VerifyAccessResponse {
  authorized: boolean;
  token?: string;
  expiresAt?: string;
  protectionLevel: ProtectionLevel;
  allowsLocalInstall: boolean;
  error?: string;
}

// ============================================================
// Team Types
// ============================================================

export interface Team {
  id: string;
  name: string;
  description: string | null;
  repositoryUrl: string | null;
  ownerId: string;
  createdAt: string;
  updatedAt: string;
}

export interface TeamSummary {
  team: Team;
  membersCount: number;
  skillsCount: number;
  recipesCount: number;
  extensionsCount: number;
  /** The current user's ID making the request */
  currentUserId: string;
}

export interface TeamMember {
  id: string;
  teamId: string;
  userId: string;
  displayName: string;
  endpointUrl: string | null;
  role: 'owner' | 'admin' | 'member';
  status: 'active' | 'invited' | 'blocked';
  joinedAt: string;
}

// ============================================================
// Skill Package Types (Agent Skills Open Standard)
// See: https://agentskills.io/specification
// ============================================================

/** Skill storage type */
export type SkillStorageType = 'inline' | 'package';

/** File in a skill package */
export interface SkillFile {
  /** Relative path within the package (e.g., "scripts/lint.py") */
  path: string;
  /** File content (text or Base64 encoded) */
  content: string;
  /** MIME content type */
  contentType: string;
  /** File size in bytes */
  size: number;
  /** Whether content is Base64 encoded */
  isBinary?: boolean;
}

/** Skill package manifest */
export interface SkillManifest {
  /** Script files (e.g., ["scripts/lint.py"]) */
  scripts: string[];
  /** Reference documentation files */
  references: string[];
  /** Asset files (templates, images, etc.) */
  assets: string[];
}

/** Extended skill metadata */
export interface SkillMetadata {
  /** Author name */
  author?: string;
  /** License (e.g., "MIT", "Apache-2.0") */
  license?: string;
  /** Homepage URL */
  homepage?: string;
  /** Repository URL */
  repository?: string;
  /** Keywords for discovery */
  keywords: string[];
  /** Estimated token usage */
  estimatedTokens?: number;
}

export interface SharedSkill {
  id: string;
  teamId: string;
  name: string;
  description: string | null;

  // Storage type and content
  storageType: SkillStorageType;

  /** Inline mode: simple text content (backward compatible) */
  content?: string;

  /** Package mode: SKILL.md content */
  skillMd?: string;

  /** Package mode: attached files */
  files?: SkillFile[];

  /** Package mode: file manifest */
  manifest?: SkillManifest;

  /** Package download URL (for large packages) */
  packageUrl?: string;

  /** Package hash for integrity verification (SHA-256) */
  packageHash?: string;

  /** Package size in bytes */
  packageSize?: number;

  /** Extended metadata from SKILL.md frontmatter */
  metadata?: SkillMetadata;

  authorId: string;
  version: string;
  visibility: 'team' | 'public';
  /** Protection level for access control */
  protectionLevel: ProtectionLevel;
  tags: string[];
  useCount: number;
  createdAt: string;
  updatedAt: string;
}

// Helper functions for skills
export const isPackageSkill = (skill: SharedSkill): boolean =>
  skill.storageType === 'package';

export const getSkillContent = (skill: SharedSkill): string | undefined =>
  skill.storageType === 'package' ? skill.skillMd : skill.content;

export const formatPackageSize = (bytes?: number): string => {
  if (!bytes) return '-';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
};

export interface SharedRecipe {
  id: string;
  teamId: string;
  name: string;
  description: string | null;
  contentYaml: string;
  category: string | null;
  authorId: string;
  version: string;
  visibility: 'team' | 'public';
  /** Protection level for access control */
  protectionLevel: ProtectionLevel;
  tags: string[];
  useCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface ExtensionConfig {
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string;
}

export interface SharedExtension {
  id: string;
  teamId: string;
  name: string;
  description: string | null;
  extensionType: 'stdio' | 'sse' | 'builtin';
  config: ExtensionConfig;
  authorId: string;
  version: string;
  visibility: 'team' | 'public';
  /** Protection level for access control */
  protectionLevel: ProtectionLevel;
  tags: string[];
  securityReviewed: boolean;
  securityNotes: string | null;
  reviewedBy: string | null;
  reviewedAt: string | null;
  useCount: number;
  createdAt: string;
  updatedAt: string;
}

/** Check if an extension is a builtin extension (cannot be deleted) */
export const isBuiltinExtension = (ext: SharedExtension): boolean =>
  ext.extensionType === 'builtin';

export interface InstallResult {
  success: boolean;
  resourceType: string;
  resourceId: string;
  installedVersion: string | null;
  localPath: string | null;
  error: string | null;
}

export interface InstalledResource {
  id: string;
  resourceType: 'skill' | 'recipe' | 'extension';
  resourceId: string;
  teamId: string;
  resourceName: string;
  localPath: string | null;
  installedVersion: string;
  latestVersion: string | null;
  hasUpdate: boolean;
  installedAt: string;
  lastCheckedAt: string | null;
  /** User who installed this resource */
  userId?: string;
  /** Authorization token */
  authorizationToken?: string;
  /** Authorization expiration time */
  authorizationExpiresAt?: string;
  /** Last verification time */
  lastVerifiedAt?: string;
  /** Protection level of the resource */
  protectionLevel: ProtectionLevel;
}

// Paginated response types
export interface PaginatedTeamsResponse {
  teams: Team[];
  total: number;
  page: number;
  limit: number;
}

export interface PaginatedMembersResponse {
  members: TeamMember[];
  total: number;
  page: number;
  limit: number;
}

export interface PaginatedSkillsResponse {
  skills: SharedSkill[];
  total: number;
  page: number;
  limit: number;
}

export interface PaginatedRecipesResponse {
  recipes: SharedRecipe[];
  total: number;
  page: number;
  limit: number;
}

export interface PaginatedExtensionsResponse {
  extensions: SharedExtension[];
  total: number;
  page: number;
  limit: number;
}

// Tab types for team detail view
export type TeamDetailTab = 'members' | 'skills' | 'recipes' | 'extensions';
