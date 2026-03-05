import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { PortalListView } from './portal/PortalListView';
import { PortalDetailView } from './portal/PortalDetailView';
import { LaboratoryGuide } from './portal/LaboratoryGuide';

interface LaboratorySectionProps {
  teamId: string;
  canManage: boolean;
}

export function LaboratorySection({ teamId, canManage }: LaboratorySectionProps) {
  const { t } = useTranslation();
  const [selectedPortalId, setSelectedPortalId] = useState<string | null>(null);
  const [tab, setTab] = useState<'workspace' | 'guide'>('workspace');

  return (
    <div className="h-full min-h-0 flex flex-col gap-3">
      <div className="rounded-xl border bg-card px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <p className="text-sm font-semibold truncate">{t('laboratory.title')}</p>
            <p className="text-caption text-muted-foreground truncate">{t('laboratory.description')}</p>
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setTab(tab === 'workspace' ? 'guide' : 'workspace')}
          >
            {tab === 'workspace' ? t('laboratory.tabs.guide') : t('laboratory.tabs.workspace')}
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1">
        {tab === 'guide' ? (
          <div className="h-full overflow-y-auto rounded-xl border bg-card">
            <LaboratoryGuide />
          </div>
        ) : selectedPortalId ? (
          <PortalDetailView
            teamId={teamId}
            portalId={selectedPortalId}
            canManage={canManage}
            onBack={() => setSelectedPortalId(null)}
          />
        ) : (
          <PortalListView
            teamId={teamId}
            canManage={canManage}
            onSelect={setSelectedPortalId}
            domain="ecosystem"
          />
        )}
      </div>
    </div>
  );
}
