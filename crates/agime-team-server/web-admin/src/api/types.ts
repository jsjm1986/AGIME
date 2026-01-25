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
}

// 前端使用的扁平化团队数据
export interface TeamWithStats extends Team {
  membersCount: number;
  skillsCount: number;
  recipesCount: number;
  extensionsCount: number;
  currentUserId: string;
}

export interface TeamMember {
  id: string;
  teamId: string;
  userId: string;
  displayName: string;
  endpointUrl: string | null;
  role: TeamRole;
  status: string;
  joinedAt: string;
}

// Shared resource types
export interface SharedSkill {
  id: string;
  teamId: string;
  name: string;
  description: string | null;
  content: string | null;
  storageType: string;
  authorId: string;
  version: string;
  visibility: string;
  protectionLevel: string;
  tags: string[];
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
  visibility: string;
  protectionLevel: string;
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
  visibility: string;
  protectionLevel: string;
  tags: string[];
  securityReviewed: boolean;
  useCount: number;
  createdAt: string;
  updatedAt: string;
}

// Invite types
export interface TeamInvite {
  id: string;
  teamId: string;
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

export interface SkillsResponse {
  skills: SharedSkill[];
}

export interface RecipesResponse {
  recipes: SharedRecipe[];
}

export interface ExtensionsResponse {
  extensions: SharedExtension[];
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
