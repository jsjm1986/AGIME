import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  Download,
  Check,
  AlertTriangle,
  MoreVertical,
  Trash2,
  Eye,
  Pencil,
  CheckCircle,
  XCircle,
  Cloud,
  Clock,
  User,
  Tag,
} from 'lucide-react';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import {
  SharedSkill,
  SharedRecipe,
  SharedExtension,
  ProtectionLevel,
  allowsLocalInstall,
} from './types';

interface ProtectionBadgeProps {
  level: ProtectionLevel;
}

const ProtectionBadge: React.FC<ProtectionBadgeProps> = ({ level }) => {
  const { t } = useTranslation('team');

  const config: Record<ProtectionLevel, { label: string; className: string }> = {
    public: {
      label: t('protectionLevel.public'),
      className: 'bg-background-muted text-text-muted',
    },
    team_installable: {
      label: t('protectionLevel.teamInstallable'),
      className: 'bg-background-muted text-text-muted',
    },
    team_online_only: {
      label: t('protectionLevel.teamOnlineOnly'),
      className: 'bg-amber-500/10 text-amber-600 dark:text-amber-400',
    },
    controlled: {
      label: t('protectionLevel.controlled'),
      className: 'bg-red-500/10 text-red-600 dark:text-red-400',
    },
  };

  const c = config[level] || config.team_installable;

  return (
    <span className={`inline-flex items-center text-xs px-2 py-0.5 rounded ${c.className}`}>
      {c.label}
    </span>
  );
};

interface ResourceCardProps {
  type: 'skill' | 'recipe' | 'extension';
  resource: SharedSkill | SharedRecipe | SharedExtension;
  onInstall?: (id: string) => void;
  onViewDetail?: () => void;
  onEdit?: () => void;
  onDelete?: () => void;
  onReview?: (approved: boolean) => void;
  isInstalling?: boolean;
  canEdit?: boolean;
  canDelete?: boolean;
  canReview?: boolean;
}

const ResourceCard: React.FC<ResourceCardProps> = ({
  type,
  resource,
  onInstall,
  onViewDetail,
  onEdit,
  onDelete,
  onReview,
  isInstalling,
  canEdit,
  canDelete,
  canReview,
}) => {
  const { t } = useTranslation('team');

  const canInstallLocally = allowsLocalInstall(resource.protectionLevel);
  const isExtension = type === 'extension';
  const ext = isExtension ? (resource as SharedExtension) : null;
  const skill = type === 'skill' ? (resource as SharedSkill) : null;
  const recipe = type === 'recipe' ? (resource as SharedRecipe) : null;

  const formatTimeAgo = (dateString: string) => {
    const date = new Date(dateString);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

    if (diffDays === 0) return t('common.today', '今天');
    if (diffDays === 1) return t('common.yesterday', '昨天');
    if (diffDays < 7) return t('common.daysAgo', { count: diffDays });
    return date.toLocaleDateString();
  };

  return (
    <div className="group relative bg-background-card rounded-xl border border-border-subtle hover:border-border-default transition-all duration-200 shadow-[0_1px_3px_rgba(0,0,0,0.08)] dark:shadow-none">
      <div className="p-4">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 mb-3">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2 flex-wrap mb-1">
              <h3 className="font-semibold text-text-default truncate">
                {resource.name}
              </h3>
              <ProtectionBadge level={resource.protectionLevel} />

              {ext && !ext.securityReviewed && (
                <span className="inline-flex items-center gap-1 text-xs text-yellow-600 dark:text-yellow-400">
                  <AlertTriangle size={12} />
                  {t('notReviewed')}
                </span>
              )}
              {ext && ext.securityReviewed && (
                <span className="inline-flex items-center gap-1 text-xs text-green-600 dark:text-green-400">
                  <Check size={12} />
                  {t('reviewed')}
                </span>
              )}
            </div>

            {resource.description && (
              <p className="text-sm text-text-muted line-clamp-2">
                {resource.description}
              </p>
            )}
          </div>

          {/* Actions dropdown */}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="ghost"
                size="sm"
                className="h-8 w-8 p-0 opacity-0 group-hover:opacity-100 transition-opacity"
              >
                <MoreVertical size={16} />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={onViewDetail}>
                <Eye size={14} />
                {t('manage.viewDetail')}
              </DropdownMenuItem>
              {canEdit && (
                <DropdownMenuItem onClick={onEdit}>
                  <Pencil size={14} />
                  {t('manage.edit')}
                </DropdownMenuItem>
              )}
              {canReview && ext && !ext.securityReviewed && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem onClick={() => onReview?.(true)}>
                    <CheckCircle size={14} className="text-green-600" />
                    {t('manage.approve')}
                  </DropdownMenuItem>
                  <DropdownMenuItem onClick={() => onReview?.(false)}>
                    <XCircle size={14} className="text-red-600" />
                    {t('manage.reject')}
                  </DropdownMenuItem>
                </>
              )}
              {canDelete && (
                <>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem variant="destructive" onClick={onDelete}>
                    <Trash2 size={14} />
                    {t('manage.delete')}
                  </DropdownMenuItem>
                </>
              )}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>

        {/* Meta info */}
        <div className="flex items-center gap-4 text-xs text-text-muted mb-4">
          <span className="flex items-center gap-1">
            <Tag size={12} />
            v{resource.version}
          </span>
          {skill && skill.tags.length > 0 && (
            <div className="flex items-center gap-1">
              {skill.tags.slice(0, 2).map((tag) => (
                <span
                  key={tag}
                  className="bg-background-muted px-1.5 py-0.5 rounded"
                >
                  {tag}
                </span>
              ))}
              {skill.tags.length > 2 && (
                <span className="text-text-muted">+{skill.tags.length - 2}</span>
              )}
            </div>
          )}
          {recipe?.category && (
            <span className="bg-background-muted px-1.5 py-0.5 rounded">
              {recipe.category}
            </span>
          )}
          {ext && (
            <span className="bg-background-muted px-1.5 py-0.5 rounded capitalize">
              {ext.extensionType}
            </span>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3 text-xs text-text-muted">
            <span className="flex items-center gap-1">
              <User size={12} />
              {resource.authorId.slice(0, 8)}
            </span>
            <span className="flex items-center gap-1">
              <Clock size={12} />
              {formatTimeAgo(resource.updatedAt)}
            </span>
          </div>

          {/* Install button */}
          {canInstallLocally ? (
            <Button
              size="sm"
              onClick={() => onInstall?.(resource.id)}
              disabled={isInstalling}
              className="h-8"
            >
              {isInstalling ? (
                <div className="animate-spin h-4 w-4 border-2 border-white border-t-transparent rounded-full" />
              ) : (
                <>
                  <Download size={14} className="mr-1" />
                  {t('install')}
                </>
              )}
            </Button>
          ) : (
            <Button
              size="sm"
              variant="outline"
              disabled
              className="h-8"
              title={t('protectionLevel.noLocalInstallTip')}
            >
              <Cloud size={14} className="mr-1" />
              {t('onlineOnly')}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
};

export default ResourceCard;
