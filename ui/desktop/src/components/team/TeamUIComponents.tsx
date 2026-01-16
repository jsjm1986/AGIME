import React from 'react';
import { useTranslation } from 'react-i18next';
import { Plus, Sparkles, Book, Puzzle, Search, Users } from 'lucide-react';
import { Button } from '../ui/button';
import { Input } from '../ui/input';

interface QuickActionsProps {
  onShareSkill: () => void;
  onShareRecipe: () => void;
  onShareExtension: () => void;
  searchValue?: string;
  onSearchChange?: (value: string) => void;
}

const QuickActions: React.FC<QuickActionsProps> = ({
  onShareSkill,
  onShareRecipe,
  onShareExtension,
  searchValue,
  onSearchChange,
}) => {
  const { t } = useTranslation('team');

  const actions = [
    {
      icon: Sparkles,
      label: t('quickActions.shareSkill', '分享技能'),
      onClick: onShareSkill,
    },
    {
      icon: Book,
      label: t('quickActions.shareRecipe', '分享预设'),
      onClick: onShareRecipe,
    },
    {
      icon: Puzzle,
      label: t('quickActions.shareExtension', '分享扩展'),
      onClick: onShareExtension,
    },
  ];

  return (
    <div className="flex items-center gap-4 px-6 py-3 border-b border-border-subtle">
      <div className="flex items-center gap-3 text-sm">
        {actions.map((action, index) => (
          <React.Fragment key={action.label}>
            {index > 0 && <span className="text-border-subtle">|</span>}
            <button
              onClick={action.onClick}
              className="flex items-center gap-1.5 text-text-muted hover:text-text-default transition-colors"
            >
              <Plus size={14} />
              <span>{action.label}</span>
            </button>
          </React.Fragment>
        ))}
      </div>

      {onSearchChange && (
        <div className="flex-1 max-w-sm ml-auto relative">
          <Search size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
          <Input
            type="text"
            value={searchValue || ''}
            onChange={(e) => onSearchChange(e.target.value)}
            placeholder={t('quickActions.searchPlaceholder', '搜索资源...')}
            className="pl-9 h-9 bg-background-default"
          />
        </div>
      )}
    </div>
  );
};

interface EmptyStateProps {
  type: 'members' | 'skills' | 'recipes' | 'extensions';
  onAction?: () => void;
}

const EmptyState: React.FC<EmptyStateProps> = ({ type, onAction }) => {
  const { t } = useTranslation('team');

  const iconMap = {
    members: Users,
    skills: Sparkles,
    recipes: Book,
    extensions: Puzzle,
  };

  const config = {
    members: {
      title: t('noMembers'),
      description: t('emptyState.membersDesc', '添加团队成员开始协作'),
      actionLabel: t('memberManage.addMember'),
    },
    skills: {
      title: t('noSkills'),
      description: t('emptyState.skillsDesc', '分享技能让团队成员都能使用'),
      actionLabel: t('share.add'),
    },
    recipes: {
      title: t('noRecipes'),
      description: t('emptyState.recipesDesc', '分享预设任务提高团队效率'),
      actionLabel: t('share.add'),
    },
    extensions: {
      title: t('noExtensions'),
      description: t('emptyState.extensionsDesc', '分享扩展增强团队能力'),
      actionLabel: t('share.add'),
    },
  };

  const c = config[type];
  const Icon = iconMap[type];

  return (
    <div className="flex flex-col items-center justify-center py-16 px-4 text-center">
      <div className="w-12 h-12 mb-4 text-text-muted">
        <Icon className="w-full h-full" />
      </div>
      <h3 className="text-lg font-medium text-text-default mb-2">{c.title}</h3>
      <p className="text-sm text-text-muted mb-6 max-w-sm">{c.description}</p>
      {onAction && (
        <Button onClick={onAction} className="gap-2">
          <Plus size={16} />
          {c.actionLabel}
        </Button>
      )}
    </div>
  );
};

interface GuideTipProps {
  type: 'skill' | 'recipe' | 'extension';
  onDismiss?: () => void;
}

const GuideTip: React.FC<GuideTipProps> = ({ type }) => {
  const { t } = useTranslation('team');

  const tips = {
    skill: t('guideTips.skill', '提示：安装团队技能后，在对话中使用 loadSkill 命令加载'),
    recipe: t('guideTips.recipe', '提示：安装预设任务后，可以快速执行常见工作流程'),
    extension: t('guideTips.extension', '提示：扩展安装后需要重启才能生效'),
  };

  return (
    <div className="mx-6 mb-4 p-3 bg-amber-500/10 border border-amber-500/20 rounded-lg">
      <p className="text-sm text-amber-700 dark:text-amber-300">{tips[type]}</p>
    </div>
  );
};

export { QuickActions, EmptyState, GuideTip };
