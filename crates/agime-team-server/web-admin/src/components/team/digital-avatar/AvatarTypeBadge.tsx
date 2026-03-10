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
        className: 'border-sky-200 bg-sky-50 text-sky-700 dark:border-sky-900/60 dark:bg-sky-950/30 dark:text-sky-300',
        icon: Globe2,
      };
    case 'internal':
      return {
        label: t('digitalAvatar.types.internal', { defaultValue: '对内执行' }),
        className: 'border-amber-200 bg-amber-50 text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/30 dark:text-amber-300',
        icon: Workflow,
      };
    case 'unknown':
    default:
      return {
        label: t('digitalAvatar.labels.unset', { defaultValue: '未配置' }),
        className: 'border-slate-200 bg-slate-50 text-slate-600 dark:border-slate-800 dark:bg-slate-900/40 dark:text-slate-300',
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
