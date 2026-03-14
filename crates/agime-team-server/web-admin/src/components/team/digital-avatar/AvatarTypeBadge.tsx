import { Globe2, HelpCircle, Workflow } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { Badge } from '../../ui/badge';
import type { UiAvatarType } from './avatarType';

function getAvatarTypeMeta(
  type: UiAvatarType,
  t: ReturnType<typeof useTranslation>['t'],
): { label: string; className: string; icon: typeof Globe2 } {
  switch (type) {
    case 'external':
      return {
        label: t('digitalAvatar.types.external', { defaultValue: '对外服务' }),
        className: 'border-[hsl(var(--status-info-text))/0.16] bg-[hsl(var(--status-info-bg))] text-[hsl(var(--status-info-text))]',
        icon: Globe2,
      };
    case 'internal':
      return {
        label: t('digitalAvatar.types.internal', { defaultValue: '对内执行' }),
        className: 'border-[hsl(var(--status-warning-text))/0.16] bg-[hsl(var(--status-warning-bg))] text-[hsl(var(--status-warning-text))]',
        icon: Workflow,
      };
    case 'unknown':
    default:
      return {
        label: t('digitalAvatar.labels.unset', { defaultValue: '未配置' }),
        className: 'border-[hsl(var(--status-neutral-text))/0.14] bg-[hsl(var(--status-neutral-bg))] text-[hsl(var(--status-neutral-text))]',
        icon: HelpCircle,
      };
  }
}

export function AvatarTypeBadge({
  type,
  className = '',
}: {
  type: UiAvatarType;
  className?: string;
}) {
  const { t } = useTranslation();
  const meta = getAvatarTypeMeta(type, t);
  const Icon = meta.icon;

  return (
    <Badge
      variant="outline"
      className={`inline-flex items-center gap-1 border text-[11px] ${meta.className} ${className}`.trim()}
    >
      <Icon className="h-3 w-3" />
      <span>{meta.label}</span>
    </Badge>
  );
}
