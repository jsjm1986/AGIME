import { useState } from 'react';
import { PortalListView } from './portal/PortalListView';
import { PortalDetailView } from './portal/PortalDetailView';

interface LaboratorySectionProps {
  teamId: string;
  canManage: boolean;
}

export function LaboratorySection({ teamId, canManage }: LaboratorySectionProps) {
  const [selectedPortalId, setSelectedPortalId] = useState<string | null>(null);

  if (selectedPortalId) {
    return (
      <PortalDetailView
        teamId={teamId}
        portalId={selectedPortalId}
        canManage={canManage}
        onBack={() => setSelectedPortalId(null)}
      />
    );
  }

  return (
    <PortalListView
      teamId={teamId}
      canManage={canManage}
      onSelect={setSelectedPortalId}
    />
  );
}
