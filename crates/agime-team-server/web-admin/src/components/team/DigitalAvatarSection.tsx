import { useTranslation } from 'react-i18next';
import { UserRound } from 'lucide-react';

interface DigitalAvatarSectionProps {
  teamId: string;
  canManage: boolean;
}

export function DigitalAvatarSection({ teamId, canManage }: DigitalAvatarSectionProps) {
  const { t } = useTranslation();

  return (
    <div className="p-6">
      <div className="max-w-3xl rounded-xl border bg-card p-6">
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <UserRound className="h-5 w-5" />
          </div>
          <div>
            <h2 className="text-lg font-semibold">{t('digitalAvatar.title')}</h2>
            <p className="text-sm text-muted-foreground">{t('digitalAvatar.description')}</p>
          </div>
        </div>

        <div className="mt-5 rounded-lg border border-dashed bg-muted/30 px-4 py-3 text-sm text-muted-foreground">
          {t('digitalAvatar.placeholder')}
        </div>

        <div className="mt-4 text-xs text-muted-foreground">
          <span className="font-medium">{t('digitalAvatar.meta.teamId')}:</span> {teamId}
          <span className="mx-2">·</span>
          <span className="font-medium">{t('digitalAvatar.meta.permission')}:</span>{' '}
          {canManage ? t('digitalAvatar.meta.manageEnabled') : t('digitalAvatar.meta.manageDisabled')}
        </div>
      </div>
    </div>
  );
}

