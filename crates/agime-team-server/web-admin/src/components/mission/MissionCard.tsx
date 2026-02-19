import { useTranslation } from 'react-i18next';
import type { MissionListItem, MissionStatus } from '../../api/mission';

interface MissionCardProps {
  mission: MissionListItem;
  onClick: (missionId: string) => void;
}

const statusColors: Record<MissionStatus, string> = {
  draft: 'bg-gray-100 text-gray-700 dark:bg-gray-800 dark:text-gray-300',
  planning: 'bg-blue-100 text-blue-700 dark:bg-blue-900 dark:text-blue-300',
  planned: 'bg-indigo-100 text-indigo-700 dark:bg-indigo-900 dark:text-indigo-300',
  running: 'bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300',
  paused: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900 dark:text-yellow-300',
  completed: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900 dark:text-emerald-300',
  failed: 'bg-red-100 text-red-700 dark:bg-red-900 dark:text-red-300',
  cancelled: 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400',
};

export function MissionCard({ mission, onClick }: MissionCardProps) {
  const { t } = useTranslation();

  return (
    <div
      onClick={() => onClick(mission.mission_id)}
      className="p-3 rounded-lg border bg-card hover:shadow-md transition-shadow cursor-pointer"
    >
      {/* Status badge */}
      <div className="flex items-center justify-between mb-2">
        <div className="flex items-center gap-1.5">
          <span className={`text-xs px-2 py-0.5 rounded-full font-medium ${statusColors[mission.status]}`}>
            {t(`mission.${mission.status}`)}
          </span>
          {mission.execution_mode === 'adaptive' && (
            <span className="text-xs px-1.5 py-0.5 rounded bg-purple-100 text-purple-700 dark:bg-purple-900 dark:text-purple-300">
              adaptive
            </span>
          )}
        </div>
        <span className="text-xs text-muted-foreground">
          {mission.agent_name}
        </span>
      </div>

      {/* Goal */}
      <p className="text-sm font-medium line-clamp-2 mb-2">{mission.goal}</p>

      {/* Progress */}
      {mission.execution_mode === 'adaptive' && mission.goal_count > 0 ? (
        <div className="mb-2">
          <div className="flex items-center justify-between text-xs text-muted-foreground mb-1">
            <span>Goals: {mission.completed_goals}/{mission.goal_count}</span>
            {mission.pivots > 0 && <span>Pivots: {mission.pivots}</span>}
          </div>
          <div className="w-full h-1.5 bg-muted rounded-full overflow-hidden">
            <div
              className="h-full bg-purple-500 rounded-full transition-all"
              style={{ width: `${(mission.completed_goals / mission.goal_count) * 100}%` }}
            />
          </div>
        </div>
      ) : mission.step_count > 0 ? (
        <div className="mb-2">
          <div className="flex items-center justify-between text-xs text-muted-foreground mb-1">
            <span>{t('mission.progress', { completed: mission.completed_steps, total: mission.step_count })}</span>
            {mission.total_tokens_used > 0 && (
              <span>{mission.total_tokens_used.toLocaleString()} tokens</span>
            )}
          </div>
          <div className="w-full h-1.5 bg-muted rounded-full overflow-hidden">
            <div
              className="h-full bg-primary rounded-full transition-all"
              style={{ width: `${mission.step_count > 0 ? (mission.completed_steps / mission.step_count) * 100 : 0}%` }}
            />
          </div>
        </div>
      ) : null}

      {/* Footer */}
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>{new Date(mission.created_at).toLocaleDateString()}</span>
        <span className="capitalize">{mission.approval_policy}</span>
      </div>
    </div>
  );
}
