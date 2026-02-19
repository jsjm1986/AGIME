// Team types
export type TeamRole = 'owner' | 'admin' | 'member';

export interface Team {
  id: string;
  name: string;
  description: string | null;
  repositoryUrl: string | null;
  ownerId: string;
  createdAt: string;
  updatedAt: string;
}

// 后端返回的团队详情响应（嵌套结构）
export interface TeamSummaryResponse {
  team: Team;
  membersCount: number;
  skillsCount: number;
  recipesCount: number;
  extensionsCount: number;
  currentUserId: string;
  currentUserRole: string;
}

// 前端使用的扁平化团队数据
export interface TeamWithStats extends Team {
  membersCount: number;
  skillsCount: number;
  recipesCount: number;
  extensionsCount: number;
  currentUserId: string;
  currentUserRole: string;
}

export interface MemberPermissions {
  canShare: boolean;
  canInstall: boolean;
  canDeleteOwn: boolean;
}

export interface TeamMember {
  id: string;
  teamId: string;
  userId: string;
  email: string;
  displayName: string;
  endpointUrl: string | null;
  role: TeamRole;
  status: string;
  permissions: MemberPermissions;
  joinedAt: string;
}

// Skill file type
export interface SkillFile {
  path: string;
  content: string;
}

// Shared resource types
export interface SharedSkill {
  id: string;
  teamId: string;
  name: string;
  description: string | null;
  content: string | null;
  storageType: string;
  skillMd: string | null;
  files: SkillFile[];
  manifest: Record<string, unknown> | null;
  packageUrl: string | null;
  packageHash: string | null;
  packageSize: number | null;
  metadata: Record<string, unknown>;
  authorId: string;
  version: string;
  previousVersionId: string | null;
  visibility: string;
  protectionLevel: string;
  dependencies: string[];
  tags: string[];
  aiDescription?: string | null;
  aiDescriptionLang?: string | null;
  aiDescribedAt?: string | null;
  useCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface SharedRecipe {
  id: string;
  teamId: string;
  name: string;
  description: string | null;
  contentYaml: string;
  category: string | null;
  authorId: string;
  version: string;
  previousVersionId: string | null;
  visibility: string;
  protectionLevel: string;
  dependencies: string[];
  tags: string[];
  useCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface SharedExtension {
  id: string;
  teamId: string;
  name: string;
  description: string | null;
  extensionType: string;
  config: Record<string, unknown>;
  authorId: string;
  version: string;
  previousVersionId: string | null;
  visibility: string;
  protectionLevel: string;
  tags: string[];
  securityReviewed: boolean;
  securityNotes: string | null;
  reviewedBy: string | null;
  reviewedAt: string | null;
  aiDescription?: string | null;
  aiDescriptionLang?: string | null;
  aiDescribedAt?: string | null;
  useCount: number;
  createdAt: string;
  updatedAt: string;
}

// Invite types
export interface TeamInvite {
  id: string;
  teamId: string;
  code: string;
  role: TeamRole;
  createdBy: string;
  expiresAt: string | null;
  maxUses: number | null;
  usedCount: number;
  createdAt: string;
}

// API Response types
export interface TeamsResponse {
  teams: Team[];
  total: number;
  page: number;
  limit: number;
}

export interface TeamResponse {
  team: TeamWithStats;
}

export interface MembersResponse {
  members: TeamMember[];
}

export interface PaginatedResponse<T> {
  items: T[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

export interface SkillsResponse {
  skills: SharedSkill[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

export interface RecipesResponse {
  recipes: SharedRecipe[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

export interface ExtensionsResponse {
  extensions: SharedExtension[];
  total: number;
  page: number;
  limit: number;
  total_pages: number;
}

export interface InvitesResponse {
  invites: TeamInvite[];
}

export interface CreateInviteResponse {
  code: string;
  url: string;
  expiresAt: string | null;
  maxUses: number | null;
  usedCount: number;
}

export interface ValidateInviteResponse {
  valid: boolean;
  teamId: string | null;
  teamName: string | null;
  role: TeamRole | null;
  expiresAt: string | null;
  error: string | null;
}

export interface AcceptInviteResponse {
  success: boolean;
  teamId: string | null;
  teamName: string | null;
  error: string | null;
}

// Smart Log types
export interface SmartLogEntry {
  id: string;
  teamId: string;
  userId: string;
  userName: string | null;
  action: string;
  resourceType: string;
  resourceId: string | null;
  resourceName: string | null;
  aiSummary: string | null;
  aiSummaryStatus: string;
  source: string;
  aiAnalysis: string | null;
  aiAnalysisStatus: string | null;
  createdAt: string;
  aiCompletedAt: string | null;
}

export interface SmartLogParams {
  resourceType?: string;
  action?: string;
  source?: string;
  userId?: string;
  page?: number;
  limit?: number;
}

export interface SmartLogsResponse {
  items: SmartLogEntry[];
  total: number;
  page: number;
  limit: number;
  totalPages: number;
}

// Team Settings types
export interface DocumentAnalysisSettings {
  enabled: boolean;
  apiUrl?: string;
  apiKeySet: boolean;
  model?: string;
  apiFormat?: string;
  agentId?: string;
  minFileSize: number;
  maxFileSize?: number | null;
  skipMimePrefixes: string[];
}

export interface TeamSettingsResponse {
  requireExtensionReview: boolean;
  membersCanInvite: boolean;
  defaultVisibility: string;
  documentAnalysis: DocumentAnalysisSettings;
}

export interface UpdateDocumentAnalysisSettings {
  enabled?: boolean;
  apiUrl?: string;
  apiKey?: string;
  model?: string;
  apiFormat?: string;
  agentId?: string;
  minFileSize?: number;
  maxFileSize?: number | null;
  skipMimePrefixes?: string[];
}

export interface UpdateTeamSettingsRequest {
  documentAnalysis?: UpdateDocumentAnalysisSettings;
}
