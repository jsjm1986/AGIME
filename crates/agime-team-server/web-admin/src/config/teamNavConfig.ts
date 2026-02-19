export interface NavItemDef {
  readonly key: string;
  readonly icon: string;
  readonly labelKey: string;
  readonly adminOnly?: boolean;
}

export const NAV_ITEMS: readonly NavItemDef[] = [
  { key: 'chat', icon: 'MessageCircle', labelKey: 'teamNav.chat' },
  { key: 'agent', icon: 'Bot', labelKey: 'teamNav.agent' },
  { key: 'documents', icon: 'FileText', labelKey: 'teamNav.documents' },
  { key: 'toolkit', icon: 'Zap', labelKey: 'teamNav.toolkit' },
  { key: 'smart-log', icon: 'ScrollText', labelKey: 'teamNav.smartLog' },
  { key: 'laboratory', icon: 'FlaskConical', labelKey: 'teamNav.laboratory' },
  { key: 'team-admin', icon: 'Users', labelKey: 'teamNav.teamAdmin', adminOnly: false },
] as const;

/** Sections within wrappers that require admin/owner role */
export const ADMIN_SECTIONS = new Set(['invites', 'settings']);

/** Label keys for sub-tabs inside wrapper components */
export const SECTION_LABEL_KEYS: Record<string, string> = {
  documents: 'documents.title',
  'agent-manage': 'teamNav.agentManage',
  chat: 'teamNav.chat',
  missions: 'teamNav.missions',
  'task-queue': 'teamNav.taskQueue',
  'smart-log': 'smartLog.title',
  laboratory: 'teamNav.laboratory',
  skills: 'teams.tabs.skills',
  recipes: 'teams.tabs.recipes',
  extensions: 'teams.tabs.extensions',
  members: 'teams.tabs.members',
  groups: 'userGroups.title',
  invites: 'teams.tabs.invites',
  settings: 'teams.tabs.settings',
};
