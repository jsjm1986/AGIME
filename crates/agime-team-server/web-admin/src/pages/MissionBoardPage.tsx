import { useState, useEffect, useCallback } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Sidebar } from '../components/layout/Sidebar';
import { MissionCard } from '../components/mission/MissionCard';
import { CreateMissionDialog } from '../components/mission/CreateMissionDialog';
import { missionApi, MissionListItem, MissionStatus } from '../api/mission';

type BoardColumn = {
  key: string;
  statuses: MissionStatus[];
  labelKey: string;
};

const columns: BoardColumn[] = [
  { key: 'active', statuses: ['planning', 'planned', 'running'], labelKey: 'mission.running' },
  { key: 'paused', statuses: ['paused'], labelKey: 'mission.paused' },
  { key: 'completed', statuses: ['completed'], labelKey: 'mission.completed' },
  { key: 'other', statuses: ['draft', 'failed', 'cancelled'], labelKey: 'mission.draft' },
];

export default function MissionBoardPage() {
  const { teamId } = useParams<{ teamId: string }>();
  const navigate = useNavigate();
  const { t } = useTranslation();

  const [missions, setMissions] = useState<MissionListItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [showCreate, setShowCreate] = useState(false);

  const loadMissions = useCallback(async () => {
    if (!teamId) return;
    try {
      const items = await missionApi.listMissions(teamId, undefined, undefined, 1, 100);
      setMissions(items || []);
    } catch (e) {
      console.error('Failed to load missions:', e);
    } finally {
      setLoading(false);
    }
  }, [teamId]);

  useEffect(() => {
    loadMissions();
  }, [loadMissions]);

  const handleCreate = async (data: Parameters<typeof missionApi.createMission>[0]) => {
    try {
      await missionApi.createMission(data);
      setShowCreate(false);
      loadMissions();
    } catch (e) {
      console.error('Failed to create mission:', e);
    }
  };

  const handleClick = (missionId: string) => {
    navigate(`/teams/${teamId}/missions/${missionId}`);
  };

  if (!teamId) return null;

  const grouped = columns.map(col => ({
    ...col,
    items: missions.filter(m => col.statuses.includes(m.status)),
  }));

  return (
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex-1 overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between p-4 border-b">
          <h1 className="text-xl font-semibold">{t('mission.title')}</h1>
          <button
            onClick={() => setShowCreate(true)}
            className="px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90"
          >
            + {t('mission.create')}
          </button>
        </div>

        {/* Board */}
        {loading ? (
          <div className="flex-1 flex items-center justify-center">
            <p className="text-muted-foreground">Loading...</p>
          </div>
        ) : missions.length === 0 ? (
          <div className="flex-1 flex items-center justify-center">
            <div className="text-center">
              <p className="text-muted-foreground mb-4">{t('mission.noMissions')}</p>
              <button
                onClick={() => setShowCreate(true)}
                className="px-4 py-2 text-sm rounded-md bg-primary text-primary-foreground hover:bg-primary/90"
              >
                + {t('mission.create')}
              </button>
            </div>
          </div>
        ) : (
          <div className="flex-1 overflow-x-auto p-4">
            <div className="flex gap-4 h-full min-w-max">
              {grouped.map(col => (
                <div key={col.key} className="w-72 flex flex-col">
                  <div className="flex items-center gap-2 mb-3">
                    <h2 className="text-sm font-semibold">{t(col.labelKey)}</h2>
                    <span className="text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded-full">
                      {col.items.length}
                    </span>
                  </div>
                  <div className="flex-1 space-y-2 overflow-y-auto">
                    {col.items.map(m => (
                      <MissionCard key={m.mission_id} mission={m} onClick={handleClick} />
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Create dialog */}
        <CreateMissionDialog
          teamId={teamId}
          open={showCreate}
          onClose={() => setShowCreate(false)}
          onCreate={handleCreate}
        />
      </main>
    </div>
  );
}
