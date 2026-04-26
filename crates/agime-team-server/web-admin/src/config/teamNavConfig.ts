export type MobileProfile =
  | 'conversation-first'
  | 'task-flow'
  | 'browse-first'
  | 'admin-depth';

export interface NavItemDef {
  readonly key: string;
  readonly icon: string;
  readonly labelKey: string;
  readonly adminOnly?: boolean;
  readonly supportsConversationMode?: boolean;
  readonly conversationPriority?: 'high' | 'medium' | 'low';
  readonly mobileProfile?: MobileProfile;
}

export const NAV_ITEMS: readonly NavItemDef[] = [
  {
    key: 'chat',
    icon: 'MessageCircle',
    labelKey: 'teamNav.chat',
    supportsConversationMode: true,
    conversationPriority: 'high',
    mobileProfile: 'conversation-first',
  },
  {
    key: 'scheduled-tasks',
    icon: 'Clock3',
    labelKey: 'teamNav.scheduledTasks',
    supportsConversationMode: true,
    conversationPriority: 'high',
    mobileProfile: 'task-flow',
  },
  {
    key: 'collaboration',
    icon: 'MessageSquareShare',
    labelKey: 'teamNav.collaboration',
    supportsConversationMode: true,
    conversationPriority: 'high',
    mobileProfile: 'conversation-first',
  },
  {
    key: 'agent',
    icon: 'Bot',
    labelKey: 'teamNav.agent',
    supportsConversationMode: true,
    conversationPriority: 'high',
    mobileProfile: 'conversation-first',
  },
  {
    key: 'documents',
    icon: 'FileText',
    labelKey: 'teamNav.documents',
    supportsConversationMode: true,
    conversationPriority: 'medium',
    mobileProfile: 'task-flow',
  },
  {
    key: 'toolkit',
    icon: 'Zap',
    labelKey: 'teamNav.toolkit',
    supportsConversationMode: true,
    conversationPriority: 'high',
    mobileProfile: 'conversation-first',
  },
  {
    key: 'smart-log',
    icon: 'ScrollText',
    labelKey: 'teamNav.smartLog',
    supportsConversationMode: true,
    conversationPriority: 'medium',
    mobileProfile: 'task-flow',
  },
  {
    key: 'ecosystem',
    icon: 'Handshake',
    labelKey: 'teamNav.ecosystem',
    supportsConversationMode: false,
    conversationPriority: 'low',
    mobileProfile: 'browse-first',
  },
  {
    key: 'experiment',
    icon: 'FlaskConical',
    labelKey: 'teamNav.experiment',
    supportsConversationMode: false,
    conversationPriority: 'low',
    mobileProfile: 'browse-first',
  },
  {
    key: 'digital-avatar',
    icon: 'UserRound',
    labelKey: 'teamNav.digitalAvatar',
    supportsConversationMode: true,
    conversationPriority: 'high',
    mobileProfile: 'conversation-first',
  },
  {
    key: 'external-users',
    icon: 'Globe',
    labelKey: 'teamNav.externalUsers',
    adminOnly: true,
    supportsConversationMode: true,
    conversationPriority: 'medium',
    mobileProfile: 'task-flow',
  },
  {
    key: 'team-admin',
    icon: 'Users',
    labelKey: 'teamNav.teamAdmin',
    adminOnly: false,
    supportsConversationMode: false,
    conversationPriority: 'low',
    mobileProfile: 'admin-depth',
  },
] as const;

/** Sections within wrappers that require admin/owner role */
export const ADMIN_SECTIONS = new Set(['invites', 'settings']);

/** Label keys for sub-tabs inside wrapper components */
export const SECTION_LABEL_KEYS: Record<string, string> = {
  documents: 'documents.title',
  'agent-manage': 'teamNav.agentManage',
  chat: 'teamNav.chat',
  'scheduled-tasks': 'teamNav.scheduledTasks',
  collaboration: 'teamNav.collaboration',
  'smart-log': 'smartLog.title',
  ecosystem: 'teamNav.ecosystem',
  experiment: 'teamNav.experiment',
  'digital-avatar': 'teamNav.digitalAvatar',
  'external-users': 'teamNav.externalUsers',
  skills: 'teams.tabs.skills',
  recipes: 'teams.tabs.recipes',
  extensions: 'teams.tabs.extensions',
  members: 'teams.tabs.members',
  groups: 'userGroups.title',
  invites: 'teams.tabs.invites',
  settings: 'teams.tabs.settings',
};
