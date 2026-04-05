import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { BookCopy, Boxes, MessageSquareText, Sparkles, Wand2 } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '../ui/tabs';
import { SkillsTab } from './SkillsTab';
import { RecipesTab } from './RecipesTab';
import { ExtensionsTab } from './ExtensionsTab';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';
import { ContextSummaryBar } from '../mobile/ContextSummaryBar';
import { MobileWorkspaceShell } from '../mobile/MobileWorkspaceShell';
import { ManagementRail } from '../mobile/ManagementRail';
import { BottomSheetPanel } from '../mobile/BottomSheetPanel';
import { Button } from '../ui/button';

interface ToolkitSectionProps {
  teamId: string;
  canManage: boolean;
}

export function ToolkitSection({ teamId, canManage }: ToolkitSectionProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { isConversationMode, isMobileWorkspace, setMode } = useMobileInteractionMode();
  const [activeTab, setActiveTab] = useState<'skills' | 'recipes' | 'extensions'>('skills');
  const [resourcePanelOpen, setResourcePanelOpen] = useState(false);

  if (isConversationMode && isMobileWorkspace) {
    const activeAreaLabel =
      activeTab === 'skills'
        ? t('teams.tabs.skills')
        : activeTab === 'recipes'
          ? t('teams.tabs.recipes')
          : t('teams.tabs.extensions');

    const activeAreaHint =
      activeTab === 'skills'
        ? t('toolkit.skillsActionHint', '适合搜索团队技能、查看已导入技能，再继续通过对话发起安装或挂载。')
        : activeTab === 'recipes'
          ? t('toolkit.recipesActionHint', '适合查找可复用配方，把它们当成 Agent 工作流模板来复用。')
          : t('toolkit.extensionsActionHint', '适合查看扩展与 MCP 真实状态，再通过对话完成安装、更新、挂载与卸载。');

    const activeAreaExamples =
      activeTab === 'skills'
        ? [
            t('toolkit.skillsExampleOne', '找一个可用的团队技能'),
            t('toolkit.skillsExampleTwo', '看看哪些技能已经导入'),
            t('toolkit.skillsExampleThree', '帮我把技能挂到当前 Agent'),
          ]
        : activeTab === 'recipes'
          ? [
              t('toolkit.recipesExampleOne', '找一个可复用的工作流模板'),
              t('toolkit.recipesExampleTwo', '看看适合文档整理的配方'),
              t('toolkit.recipesExampleThree', '帮我用这个配方继续推进任务'),
            ]
          : [
              t('toolkit.extensionsExampleOne', '装一个 Playwright MCP'),
              t('toolkit.extensionsExampleTwo', '看看当前扩展真实状态'),
              t('toolkit.extensionsExampleThree', '把扩展挂到指定 Agent'),
            ];

    const resourceCards: Array<{
      tab: 'skills' | 'recipes' | 'extensions';
      title: string;
      description: string;
      icon: typeof Sparkles;
    }> = [
      {
        tab: 'skills',
        title: t('teams.tabs.skills'),
        description: t('toolkit.skillsHint', '搜索团队技能、查看已导入技能，并继续通过对话发起安装或挂载。'),
        icon: Sparkles,
      },
      {
        tab: 'recipes',
        title: t('teams.tabs.recipes'),
        description: t('toolkit.recipesHint', '查看可复用配方，把它们当成 Agent 工作流模板来复用。'),
        icon: BookCopy,
      },
      {
        tab: 'extensions',
        title: t('teams.tabs.extensions'),
        description: t('toolkit.extensionsHint', '查看扩展与 MCP 真实状态，再通过对话完成安装、更新、挂载与卸载。'),
        icon: Boxes,
      },
    ];

    return (
      <MobileWorkspaceShell
        summary={
          <ContextSummaryBar
            eyebrow={t('teamNav.toolkit')}
            title={t('toolkit.mobileTitle', '团队资源供给台')}
            description={t(
              'toolkit.mobileDescription',
              '先告诉 Agent 你要什么资源，再回来确认真实状态与挂载结果。',
            )}
            badge={(
              <span className="rounded-full border border-[hsl(var(--semantic-extension))]/30 bg-[hsl(var(--semantic-extension))]/10 px-2 py-0.5 text-[9px] font-semibold tracking-[0.08em] text-[hsl(var(--semantic-extension))] uppercase">
                {activeAreaLabel}
              </span>
            )}
            metrics={[
              {
                label: t('toolkit.nextStep', '下一步'),
                value: t('toolkit.startFromConversation', '从对话发起动作'),
              },
            ]}
          />
        }
        actions={
          <div className="grid grid-cols-3 gap-1.5">
            <Button
              variant={activeTab === 'skills' ? 'default' : 'outline'}
              size="sm"
              className="h-8 rounded-full px-2.5 text-[10px] font-semibold"
              onClick={() => setActiveTab('skills')}
            >
              <Sparkles className="h-3.5 w-3.5" />
              {t('teams.tabs.skills')}
            </Button>
            <Button
              variant={activeTab === 'recipes' ? 'default' : 'outline'}
              size="sm"
              className="h-8 rounded-full px-2.5 text-[10px] font-semibold"
              onClick={() => setActiveTab('recipes')}
            >
              <BookCopy className="h-3.5 w-3.5" />
              {t('teams.tabs.recipes')}
            </Button>
            <Button
              variant={activeTab === 'extensions' ? 'default' : 'outline'}
              size="sm"
              className="h-8 rounded-full px-2.5 text-[10px] font-semibold"
              onClick={() => setActiveTab('extensions')}
            >
              <Boxes className="h-3.5 w-3.5" />
              {t('teams.tabs.extensions')}
            </Button>
          </div>
        }
        stage={(
          <div className="flex h-full min-h-[380px] flex-col gap-3 p-3">
            <div className="rounded-[24px] border border-[hsl(var(--semantic-extension))]/18 bg-[linear-gradient(180deg,hsl(var(--semantic-extension))/0.07_0%,transparent_100%)] px-3.5 py-3.5">
              <div className="flex items-start gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-[16px] bg-[hsl(var(--semantic-extension))]/12 text-[hsl(var(--semantic-extension))]">
                  <Wand2 className="h-4.5 w-4.5" />
                </div>
                <div className="min-w-0 flex-1">
                  <div className="text-[9px] font-semibold tracking-[0.16em] text-muted-foreground uppercase">
                    {t('toolkit.agentFirstLabel', 'Agent-first 资源供给')}
                  </div>
                  <h3 className="mt-1 text-[15px] font-semibold tracking-[-0.025em] text-foreground">
                    {t('toolkit.stageTitle', '先说目标，让 Agent 帮你找资源和完成动作')}
                  </h3>
                  <p className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                    {activeAreaHint}
                  </p>
                </div>
              </div>

              <div className="mt-3 flex flex-wrap gap-1.5">
                {activeAreaExamples.map((example) => (
                  <span
                    key={example}
                    className="rounded-full border border-border/55 bg-background/72 px-2.5 py-1 text-[10px] leading-4 text-muted-foreground"
                  >
                    {example}
                  </span>
                ))}
              </div>

              <div className="mt-3 space-y-2">
                <Button
                  className="h-10 w-full justify-center rounded-[16px] text-[11px] font-semibold"
                  onClick={() => navigate(`/teams/${teamId}?section=collaboration`)}
                >
                  <MessageSquareText className="mr-1.5 h-4 w-4" />
                  {t('toolkit.stageActionChat', '进入智能协作发起资源动作')}
                </Button>
                <div className="grid grid-cols-2 gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    className="h-9 rounded-[14px] text-[11px]"
                    onClick={() => setResourcePanelOpen(true)}
                  >
                    <Boxes className="mr-1.5 h-3.5 w-3.5" />
                    {t('toolkit.openResources', '打开资源面板')}
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-9 rounded-[14px] text-[11px] text-muted-foreground"
                    onClick={() => void setMode('classic')}
                  >
                    {t('toolkit.switchClassic', '切到经典模式')}
                  </Button>
                </div>
              </div>
            </div>
          </div>
        )}
        rail={(
          <ManagementRail
            title={t('toolkit.managementTitle', '当前供给区')}
            description={t(
              'toolkit.managementHint',
              '只保留当前焦点和真实状态，详细资源入口放进面板里。',
            )}
          >
            <div className="space-y-1.5 rounded-[16px] border border-border/60 bg-background/82 px-3 py-3 text-[11px]">
              <div className="flex items-start justify-between gap-3">
                <span className="text-muted-foreground">{t('toolkit.activeArea', '焦点资源')}</span>
                <span className="text-right font-semibold text-foreground">{activeAreaLabel}</span>
              </div>
              <div className="flex items-start justify-between gap-3">
                <span className="text-muted-foreground">{t('toolkit.primaryFlow', '处理方式')}</span>
                <span className="text-right font-semibold text-foreground">
                  {t('toolkit.conversationPreferred', '对话驱动')}
                </span>
              </div>
              <Button
                variant="outline"
                size="sm"
                className="mt-2 h-8 w-full justify-center rounded-[12px] text-[11px]"
                onClick={() => setResourcePanelOpen(true)}
              >
                {t('toolkit.openResources', '打开资源面板')}
              </Button>
            </div>
          </ManagementRail>
        )}
        panel={(
          <BottomSheetPanel
            open={resourcePanelOpen}
            onOpenChange={setResourcePanelOpen}
            title={t('toolkit.managementTitle', '资源入口')}
            description={t(
              'toolkit.managementHint',
              '先选供给区，再回到对话里继续发起安装、挂载、更新或卸载。',
            )}
          >
            <div className="space-y-3">
              <div className="space-y-2">
                {resourceCards.map((card) => {
                  const Icon = card.icon;
                  const isActive = activeTab === card.tab;
                  return (
                    <button
                      key={card.tab}
                      type="button"
                      className={`w-full rounded-[18px] border px-3.5 py-3 text-left transition-colors ${isActive ? 'border-primary/35 bg-primary/6' : 'border-border/65 bg-background/88 hover:bg-accent/20'}`}
                      onClick={() => setActiveTab(card.tab)}
                    >
                      <div className="flex items-start gap-3">
                        <div className={`flex h-9 w-9 items-center justify-center rounded-[14px] ${isActive ? 'bg-primary/12 text-primary' : 'bg-[hsl(var(--ui-surface-panel-muted))/0.7] text-muted-foreground'}`}>
                          <Icon className="h-4 w-4" />
                        </div>
                        <div className="min-w-0 flex-1">
                          <div className="flex items-center gap-2">
                            <span className="truncate text-[12px] font-semibold text-foreground">{card.title}</span>
                            {isActive ? (
                              <span className="rounded-full bg-primary/10 px-2 py-0.5 text-[10px] font-semibold text-primary">
                                {t('toolkit.currentSupplyLane', '当前焦点')}
                              </span>
                            ) : null}
                          </div>
                          <p className="mt-1 line-clamp-2 text-[10px] leading-4.5 text-muted-foreground">
                            {card.description}
                          </p>
                        </div>
                      </div>
                    </button>
                  );
                })}
              </div>

              <div className="rounded-[18px] border border-border/65 bg-background/88 px-3.5 py-3">
                <div className="text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
                  {t('toolkit.primaryFlow', '建议动作')}
                </div>
                <p className="mt-1 text-[11px] leading-4.5 text-muted-foreground">
                  {activeAreaHint}
                </p>
              </div>

              <div className="space-y-2">
                <Button
                  className="h-10 w-full rounded-[16px] text-[11px] font-semibold"
                  onClick={() => {
                    setResourcePanelOpen(false);
                    navigate(`/teams/${teamId}?section=collaboration`);
                  }}
                >
                  <MessageSquareText className="mr-1.5 h-3.5 w-3.5" />
                  {t('toolkit.stageActionChat', '进入智能协作发起资源动作')}
                </Button>
                <Button
                  variant="outline"
                  className="h-9 w-full rounded-[14px] text-[11px]"
                  onClick={() => {
                    setResourcePanelOpen(false);
                    void setMode('classic');
                  }}
                >
                  {t('toolkit.switchClassic', '切到经典模式')}
                </Button>
              </div>
            </div>
          </BottomSheetPanel>
        )}
      />
    );
  }

  return (
    <Tabs defaultValue="skills">
      <TabsList>
        <TabsTrigger value="skills">{t('teams.tabs.skills')}</TabsTrigger>
        <TabsTrigger value="recipes">{t('teams.tabs.recipes')}</TabsTrigger>
        <TabsTrigger value="extensions">{t('teams.tabs.extensions')}</TabsTrigger>
      </TabsList>
      <TabsContent value="skills">
        <SkillsTab teamId={teamId} canManage={canManage} />
      </TabsContent>
      <TabsContent value="recipes">
        <RecipesTab teamId={teamId} canManage={canManage} />
      </TabsContent>
      <TabsContent value="extensions">
        <ExtensionsTab teamId={teamId} canManage={canManage} />
      </TabsContent>
    </Tabs>
  );
}
