import { useTranslation } from 'react-i18next';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs';
import { MembersTab } from './MembersTab';
import { UserGroupsTab } from './UserGroupsTab';
import { InvitesTab } from './InvitesTab';
import { SettingsTab } from './SettingsTab';
import type { TeamWithStats, TeamRole } from '../../api/types';

interface TeamAdminSectionProps {
  team: TeamWithStats;
  userRole: TeamRole;
  canManage: boolean;
  onUpdate: () => void;
  onDelete: () => void;
}

export function TeamAdminSection({ team, userRole, canManage, onUpdate, onDelete }: TeamAdminSectionProps) {
  const { t } = useTranslation();

  return (
    <Tabs defaultValue="members">
      <TabsList>
        <TabsTrigger value="members">{t('teams.tabs.members')}</TabsTrigger>
        <TabsTrigger value="groups">{t('userGroups.title')}</TabsTrigger>
        {canManage && (
          <TabsTrigger value="invites">{t('teams.tabs.invites')}</TabsTrigger>
        )}
        {canManage && (
          <TabsTrigger value="settings">{t('teams.tabs.settings')}</TabsTrigger>
        )}
      </TabsList>
      <TabsContent value="members">
        <MembersTab teamId={team.id} userRole={userRole} onUpdate={onUpdate} />
      </TabsContent>
      <TabsContent value="groups">
        <UserGroupsTab teamId={team.id} />
      </TabsContent>
      {canManage && (
        <TabsContent value="invites">
          <InvitesTab teamId={team.id} />
        </TabsContent>
      )}
      {canManage && (
        <TabsContent value="settings">
          <SettingsTab team={team} onUpdate={onUpdate} onDelete={onDelete} />
        </TabsContent>
      )}
    </Tabs>
  );
}
