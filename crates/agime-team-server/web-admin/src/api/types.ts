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

export interface SkillRegistrySearchItem {
  id: string;
  skill_id: string;
  name: string;
  source: string;
  installs: number;
  is_duplicate: boolean;
  supports_preview: boolean;
  supports_import: boolean;
}

export interface SkillRegistrySearchResponse {
  query: string;
  team_id: string;
  count: number;
  skills: SkillRegistrySearchItem[];
}

export interface SkillRegistryPreviewFile {
  path: string;
}

export interface SkillRegistryPreviewResponse {
  team_id: string;
  source: string;
  skill_id: string;
  source_ref: string;
  source_commit: string;
  skill_dir: string;
  name: string;
  description: string;
  tags: string[];
  already_imported: boolean;
  skill_md: string;
  truncated: boolean;
  files: SkillRegistryPreviewFile[];
  skipped_files: string[];
}

export interface SkillRegistryImportResponse {
  team_id: string;
  source: string;
  skill_id: string;
  source_ref: string;
  source_commit: string;
  imported_skill_id: string;
  name: string;
  description: string | null;
  visibility: string;
  file_count: number;
  skipped_files: string[];
}

export interface ImportedRegistrySkillSummary {
  imported_skill_id: string;
  name: string;
  description: string | null;
  version: string;
  visibility: string;
  source: string;
  skill_id: string;
  source_ref: string;
  source_commit: string | null;
  source_tree_sha: string | null;
  source_url: string | null;
  registry_provider: string | null;
  skipped_files: string[];
  updated_at: string;
}

export interface ImportedRegistrySkillsResponse {
  team_id: string;
  count: number;
  skills: ImportedRegistrySkillSummary[];
}

export interface SkillRegistryUpdateInspection {
  imported_skill_id: string;
  name: string;
  current_version: string;
  description: string | null;
  source: string;
  skill_id: string;
  source_ref: string;
  current_tree_sha: string;
  latest_tree_sha: string;
  latest_source_commit: string;
  has_update: boolean;
  owner: string;
  repo: string;
}

export interface SkillRegistryUpdatesResponse {
  team_id: string;
  count: number;
  updates: SkillRegistryUpdateInspection[];
}

export interface SkillRegistryUpgradeResponse {
  team_id: string;
  imported_skill_id: string;
  name: string;
  upgraded: boolean;
  reason?: string;
  previous_version?: string;
  current_version: string;
  source_ref: string;
  source_commit?: string;
  source_tree_sha: string;
  file_count?: number;
  skipped_files?: string[];
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

export interface AiDescribeSettings {
  agentId?: string;
}

export interface GeneralAgentSettings {
  defaultAgentId?: string;
}

export interface ChatAssistantSettings {
  companyName?: string;
  departmentName?: string;
  teamName?: string;
  teamSummary?: string;
  businessContext?: string;
  toneHint?: string;
}

export type ShellSecurityMode = 'off' | 'warn' | 'block';

export interface ShellSecuritySettings {
  mode: ShellSecurityMode;
}

export type AvatarGovernanceRiskAction = 'auto_execute' | 'manager_review' | 'human_review';
export type AvatarGovernanceManagerApprovalMode = 'manager_decides' | 'human_gate';
export type AvatarGovernanceOptimizationMode = 'dual_loop' | 'manager_only';

export interface AvatarGovernanceTeamSettings {
  autoProposalTriggerCount: number;
  managerApprovalMode: AvatarGovernanceManagerApprovalMode;
  optimizationMode: AvatarGovernanceOptimizationMode;
  lowRiskAction: AvatarGovernanceRiskAction;
  mediumRiskAction: AvatarGovernanceRiskAction;
  highRiskAction: AvatarGovernanceRiskAction;
  autoCreateCapabilityRequests: boolean;
  autoCreateOptimizationTickets: boolean;
  requireHumanForPublish: boolean;
}

export interface TeamSettingsResponse {
  requireExtensionReview: boolean;
  membersCanInvite: boolean;
  defaultVisibility: string;
  documentAnalysis: DocumentAnalysisSettings;
  aiDescribe: AiDescribeSettings;
  generalAgent: GeneralAgentSettings;
  chatAssistant: ChatAssistantSettings;
  shellSecurity: ShellSecuritySettings;
  avatarGovernance: AvatarGovernanceTeamSettings;
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
  aiDescribe?: AiDescribeSettings;
  generalAgent?: GeneralAgentSettings;
  chatAssistant?: Partial<ChatAssistantSettings>;
  shellSecurity?: Partial<ShellSecuritySettings>;
  avatarGovernance?: Partial<AvatarGovernanceTeamSettings>;
}
