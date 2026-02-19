import { useTranslation } from 'react-i18next';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs';
import { AgentManagePanel } from './AgentManagePanel';
import { MissionsPanel } from './MissionsPanel';
import { TaskQueuePanel } from './TaskQueuePanel';
import type { TeamAgent } from '../../api/agent';

interface AgentSectionProps {
  teamId: string;
  onOpenChat: (agent: TeamAgent) => void;
}

export function AgentSection({ teamId, onOpenChat }: AgentSectionProps) {
  const { t } = useTranslation();

  return (
    <Tabs defaultValue="agent-manage">
      <TabsList>
        <TabsTrigger value="agent-manage">{t('teamNav.agentManage')}</TabsTrigger>
        <TabsTrigger value="missions">{t('teamNav.missions')}</TabsTrigger>
        <TabsTrigger value="task-queue">{t('teamNav.taskQueue')}</TabsTrigger>
      </TabsList>
      <TabsContent value="agent-manage">
        <AgentManagePanel teamId={teamId} onOpenChat={onOpenChat} />
      </TabsContent>
      <TabsContent value="missions">
        <MissionsPanel teamId={teamId} />
      </TabsContent>
      <TabsContent value="task-queue">
        <TaskQueuePanel teamId={teamId} />
      </TabsContent>
    </Tabs>
  );
}
