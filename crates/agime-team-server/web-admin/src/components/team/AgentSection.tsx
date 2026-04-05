import { useTranslation } from 'react-i18next';
import { Bot } from 'lucide-react';
import { AgentManagePanel } from './AgentManagePanel';
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
  initialTab?: 'agent-manage';
}

export function AgentSection({ teamId, onOpenChat, onOpenDigitalAvatar }: AgentSectionProps) {
  const { t } = useTranslation();
  const { isConversationMode, isMobileWorkspace } = useMobileInteractionMode();

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
                value: t('teamNav.agentManage'),
              },
            ]}
          />
        }
        actions={
          <div className="grid grid-cols-1 gap-2">
            <Button
              variant="default"
              size="sm"
              className="h-10 rounded-[16px] px-3"
            >
              <Bot className="h-3.5 w-3.5" />
              {t('teamNav.agentManage')}
            </Button>
          </div>
        }
        stage={
          <AgentManagePanel
            teamId={teamId}
            onOpenChat={onOpenChat}
            onOpenDigitalAvatar={onOpenDigitalAvatar}
          />
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
                  {t('teamNav.agentManage')}
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
    <AgentManagePanel
      teamId={teamId}
      onOpenChat={onOpenChat}
      onOpenDigitalAvatar={onOpenDigitalAvatar}
    />
  );
}
