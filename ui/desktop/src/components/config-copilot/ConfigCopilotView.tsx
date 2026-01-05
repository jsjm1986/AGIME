import { useState, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Puzzle, Server, Stethoscope, FileJson, ArrowRight, Loader2 } from 'lucide-react';
import { createConfigCopilotRecipe } from './configCopilotPrompt';
import { encodeRecipe } from '../../recipe';
import { startNewSession } from '../../sessions';
import { useNavigation } from '../../hooks/useNavigation';
import { useIsMobile } from '../../hooks/use-mobile';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import { SettingsCard } from '../settings/common';
import { ScrollArea } from '../ui/scroll-area';
import { toastService } from '../../toasts';
import { cn } from '../../utils';

/**
 * ConfigCopilotView - 配置助手欢迎页面
 *
 * 使用 Recipe 系统启动会话：
 * - instructions 字段作为系统级提示词（用户不可见）
 * - 用户消息通过 initialText 传递（用户可见，简短）
 *
 * 响应式设计：
 * - 移动端：单列布局，居中标题，触控友好
 * - 桌面端：双列卡片网格，左对齐标题
 */
export default function ConfigCopilotView() {
  const { t } = useTranslation('configCopilot');
  const setView = useNavigation();
  const isMobile = useIsMobile();
  const [isLoading, setIsLoading] = useState(false);

  // 使用 ref 作为同步锁，防止快速多次点击造成的竞态条件
  // useState 是异步更新的，但 ref.current 是立即更新的
  const isStartingRef = useRef(false);

  const handleStartChat = async (topic?: string) => {
    // 使用 ref 进行同步检查，防止快速多次点击
    if (isStartingRef.current || isLoading) return;
    isStartingRef.current = true;
    setIsLoading(true);

    try {
      // 创建 Config Copilot Recipe
      // - instructions: 包含配置知识（用户不可见，作为系统提示词）
      // - prompt: 预填充输入框的建议内容（用户可见，可修改后发送）
      const recipe = createConfigCopilotRecipe(topic);

      // 编码为 deeplink
      const deeplink = await encodeRecipe(recipe);

      // 使用 Recipe deeplink 启动新会话
      // 不传 initialMessage，这样不会自动发送
      // Recipe 的 prompt 会预填充到输入框，用户可以修改后发送
      await startNewSession(undefined, setView, {
        recipeDeeplink: deeplink,
      });
      // 注意：成功后不重置 isStartingRef，因为页面会导航离开
    } catch (error) {
      console.error('Failed to start Config Copilot session:', error);
      // 显示错误 Toast
      toastService.error({
        title: t('errors.startFailed'),
        msg: t('errors.startFailedDesc'),
        traceback: error instanceof Error ? error.message : String(error),
      });
      // 只在失败时重置状态，允许用户重试
      isStartingRef.current = false;
      setIsLoading(false);
    }
  };

  // 功能卡片配置 - topic 包含详细的提示模板，预填充到输入框
  const featureCards = [
    {
      icon: <Puzzle className="h-5 w-5" />,
      titleKey: 'features.installExtension.title',
      descKey: 'features.installExtension.desc',
      // 预填充的建议内容，用户可以修改扩展名称后发送
      topic: t('prompts.installExtension'),
    },
    {
      icon: <Server className="h-5 w-5" />,
      titleKey: 'features.addProvider.title',
      descKey: 'features.addProvider.desc',
      topic: t('prompts.addProvider'),
    },
    {
      icon: <Stethoscope className="h-5 w-5" />,
      titleKey: 'features.diagnose.title',
      descKey: 'features.diagnose.desc',
      topic: t('prompts.diagnose'),
    },
    {
      icon: <FileJson className="h-5 w-5" />,
      titleKey: 'features.editConfig.title',
      descKey: 'features.editConfig.desc',
      topic: t('prompts.editConfig'),
    },
  ];

  return (
    <MainPanelLayout>
      <div className="flex-1 flex flex-col min-h-0">
        {/* Header - 响应式布局 */}
        <div
          className={cn(
            'bg-background-default',
            isMobile ? 'px-4 pt-14 pb-4' : 'px-8 pb-6 pt-16'
          )}
        >
          <div
            className={cn(
              'flex flex-col page-transition',
              isMobile && 'items-center text-center'
            )}
          >
            <h1
              className={cn(
                'font-light text-text-default',
                isMobile ? 'text-2xl' : 'text-4xl'
              )}
            >
              {t('title')}
            </h1>
            <p
              className={cn(
                'text-text-muted mt-2',
                isMobile ? 'text-sm' : 'max-w-2xl'
              )}
            >
              {t('subtitle')}
            </p>
          </div>
        </div>

        {/* Content - min-h-0 确保移动端滚动正常工作 */}
        <ScrollArea className="flex-1 min-h-0">
          <div
            className={cn(
              'pb-8 space-y-6',
              isMobile ? 'px-4' : 'px-8'
            )}
          >
            {/* 功能卡片 - 响应式网格 */}
            <div
              className={cn(
                'grid gap-4',
                isMobile ? 'grid-cols-1' : 'grid-cols-1 lg:grid-cols-2'
              )}
            >
              {featureCards.map((card, index) => (
                <div
                  key={index}
                  onClick={() => handleStartChat(card.topic)}
                  className={cn(
                    'cursor-pointer group',
                    isLoading && 'pointer-events-none opacity-50'
                  )}
                >
                  <SettingsCard
                    icon={card.icon}
                    title={t(card.titleKey)}
                    description={t(card.descKey)}
                    className={cn(
                      'h-full transition-all duration-200',
                      'hover:border-border-hover hover:shadow-md group-hover:bg-background-hover',
                      // 移动端触控优化：更大的点击区域和视觉反馈
                      isMobile && 'active:scale-[0.98] active:bg-background-hover'
                    )}
                  >
                    <div className="flex items-center justify-end text-sm text-text-muted group-hover:text-text-default transition-colors">
                      <span>{t('startWithThis')}</span>
                      <ArrowRight className="w-4 h-4 ml-1 transition-transform group-hover:translate-x-1" />
                    </div>
                  </SettingsCard>
                </div>
              ))}
            </div>

            {/* 工作原理卡片 */}
            <SettingsCard
              title={t('howItWorks.title')}
              description={t('howItWorks.description')}
            >
              <div
                className={cn(
                  'grid gap-4',
                  isMobile ? 'grid-cols-1' : 'grid-cols-1 md:grid-cols-3'
                )}
              >
                {[1, 2, 3].map((step) => (
                  <div key={step} className="flex items-start gap-3">
                    <div className="flex-shrink-0 w-6 h-6 rounded-full bg-text-muted/10 flex items-center justify-center text-xs font-medium text-text-muted">
                      {step}
                    </div>
                    <p className="text-sm text-text-muted">
                      {t(`howItWorks.step${step}`)}
                    </p>
                  </div>
                ))}
              </div>
            </SettingsCard>

            {/* 自由对话按钮 */}
            <div
              className={cn(
                'flex flex-col items-center',
                isMobile ? 'py-4' : 'py-6'
              )}
            >
              <p className="text-sm text-text-muted mb-4 text-center">
                {t('orFreeChat')}
              </p>
              <Button
                onClick={() => handleStartChat()}
                variant="outline"
                size={isMobile ? 'default' : 'lg'}
                className={cn(
                  isMobile ? 'w-full max-w-xs' : 'px-8',
                  // 移动端触控优化
                  isMobile && 'min-h-[44px]'
                )}
                disabled={isLoading}
              >
                {isLoading ? (
                  <Loader2 className="w-4 h-4 mr-2 animate-spin" />
                ) : null}
                {t('startButton')}
                {!isLoading && <ArrowRight className="w-4 h-4 ml-2" />}
              </Button>
            </div>
          </div>
        </ScrollArea>
      </div>
    </MainPanelLayout>
  );
}
