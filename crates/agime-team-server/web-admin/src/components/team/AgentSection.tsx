import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot, ListChecks, Workflow } from 'lucide-react';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs';
import { AgentManagePanel } from './AgentManagePanel';
import { MissionsPanel } from './MissionsPanel';
import { TaskQueuePanel } from './TaskQueuePanel';
import type { TeamAgent } from '../../api/agent';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { ContextSummaryBar } from '../mobile/ContextSummaryBar';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';
import { ManagementRail } from '../mobile/ManagementRail';
import { Button } from '../ui/button';

interface AgentSectionProps {
  teamId: string;
  onOpenChat: (agent: TeamAgent) => void;
  onOpenDigitalAvatar?: () => void;
  initialTab?: 'agent-manage' | 'missions' | 'task-queue';
}

export function AgentSection({ teamId, onOpenChat, onOpenDigitalAvatar, initialTab = 'agent-manage' }: AgentSectionProps) {
  const { t } = useTranslation();
  const { isConversationMode, isMobileWorkspace } = useMobileInteractionMode();
  const [activeTab, setActiveTab] = useState<'agent-manage' | 'missions' | 'task-queue'>(initialTab);

  useEffect(() => {
    setActiveTab(initialTab);
  }, [initialTab]);

  if (isConversationMode && isMobileWorkspace && activeTab === 'missions') {
    return <MissionsPanel teamId={teamId} />;
  }

  if (isConversationMode && isMobileWorkspace) {
    return (
      <MobileWorkspaceShell
        summary={
          <ContextSummaryBar
            eyebrow={t('teamNav.agent')}
            title={t('agent.mobileTitle', 'Agent 工作区')}
            description={t(
              'agent.mobileDescription',
              '移动对话模式优先让 Agent 处理任务与配置协同，管理面板只承担确认、查看和兜底操作。',
            )}
            metrics={[
              {
                label: t('agent.primaryFlow', '主要方式'),
                value: t('agent.conversationPreferred', '对话协同'),
              },
              {
                label: t('agent.activeArea', '当前区域'),
                value: activeTab === 'agent-manage'
                  ? t('teamNav.agentManage')
                  : activeTab === 'missions'
                    ? t('teamNav.missions')
                    : t('teamNav.taskQueue'),
              },
            ]}
          />
        }
        actions={
          <div className="grid grid-cols-3 gap-2">
            <Button
              variant={activeTab === 'agent-manage' ? 'default' : 'outline'}
              size="sm"
              className="h-10 rounded-[16px] px-3"
              onClick={() => setActiveTab('agent-manage')}
            >
              <Bot className="h-3.5 w-3.5" />
              {t('teamNav.agentManage')}
            </Button>
            <Button
              variant={activeTab === 'missions' ? 'default' : 'outline'}
              size="sm"
              className="h-10 rounded-[16px] px-3"
              onClick={() => setActiveTab('missions')}
            >
              <Workflow className="h-3.5 w-3.5" />
              {t('teamNav.missions')}
            </Button>
            <Button
              variant={activeTab === 'task-queue' ? 'default' : 'outline'}
              size="sm"
              className="h-10 rounded-[16px] px-3"
              onClick={() => setActiveTab('task-queue')}
            >
              <ListChecks className="h-3.5 w-3.5" />
              {t('teamNav.taskQueue')}
            </Button>
          </div>
        }
        stage={
          activeTab === 'agent-manage' ? (
            <AgentManagePanel
              teamId={teamId}
              onOpenChat={onOpenChat}
              onOpenDigitalAvatar={onOpenDigitalAvatar}
            />
          ) : (
            <TaskQueuePanel teamId={teamId} />
          )
        }
        rail={
          <ManagementRail
            title={t('agent.managementTitle', 'Agent 管理')}
            description={t(
              'agent.managementHint',
              '管理和队列退到辅助层；任务执行单元会单独成为主舞台，保持 Agent-first 的工作方式。',
            )}
          >
            <div className="grid grid-cols-2 gap-3">
              <div className="rounded-[18px] border border-border/60 bg-background/80 px-4 py-3">
                <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/72">
                  {t('agent.activeArea', '当前区域')}
                </div>
                <div className="mt-1 text-[13px] font-semibold text-foreground">
                  {activeTab === 'agent-manage' ? t('teamNav.agentManage') : t('teamNav.taskQueue')}
                </div>
              </div>
              <div className="rounded-[18px] border border-border/60 bg-background/80 px-4 py-3">
                <div className="text-[10px] uppercase tracking-[0.12em] text-muted-foreground/72">
                  {t('agent.primaryFlow', '主要方式')}
                </div>
                <div className="mt-1 text-[13px] font-semibold text-foreground">
                  {t('agent.conversationPreferred', '对话协同')}
                </div>
              </div>
            </div>
          </ManagementRail>
        }
      />
    );
  }

  return (
    <Tabs defaultValue="agent-manage">
      <TabsList>
        <TabsTrigger value="agent-manage">{t('teamNav.agentManage')}</TabsTrigger>
        <TabsTrigger value="missions">{t('teamNav.missions')}</TabsTrigger>
        <TabsTrigger value="task-queue">{t('teamNav.taskQueue')}</TabsTrigger>
      </TabsList>
      <TabsContent value="agent-manage">
        <AgentManagePanel
          teamId={teamId}
          onOpenChat={onOpenChat}
          onOpenDigitalAvatar={onOpenDigitalAvatar}
        />
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
