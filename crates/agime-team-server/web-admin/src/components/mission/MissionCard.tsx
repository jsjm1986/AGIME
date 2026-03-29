import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import type { MissionListItem, MissionStatus } from '../../api/mission';
import { StatusBadge, MISSION_STATUS_MAP } from '../ui/status-badge';
import { formatDate } from '../../utils/format';

interface MissionCardProps {
  mission: MissionListItem;
  onClick: (missionId: string) => void;
  onQuickStart?: (missionId: string) => void;
  onQuickPause?: (missionId: string) => void;
}

// Status dot colors for inline indicators
const statusDot: Record<MissionStatus, string> = {
  draft: 'bg-[hsl(var(--muted-foreground))/0.48]',
  planning: 'bg-status-info-text animate-pulse',
  planned: 'bg-primary',
  running: 'bg-status-success-text animate-pulse',
  paused: 'bg-status-warning-text',
  completed: 'bg-status-success-text',
  failed: 'bg-status-error-text',
  cancelled: 'bg-[hsl(var(--muted-foreground))/0.48]',
};

// --- Shared helpers ---

interface MissionProgress {
  isAdaptive: boolean;
  completed: number;
  total: number;
  pct: number;
}

function computeProgress(mission: MissionListItem): MissionProgress {
  const isAdaptive = mission.goal_count > 0;
  const completed = isAdaptive ? mission.completed_goals : mission.completed_steps;
  const total = isAdaptive ? mission.goal_count : mission.step_count;
  const pct = total > 0 ? Math.round((completed / total) * 100) : 0;
  return { isAdaptive, completed, total, pct };
}

function ProgressBar({ total, pct, isAdaptive }: Pick<MissionProgress, 'total' | 'pct' | 'isAdaptive'>) {
  if (total <= 0) return null;
  return (
    <div className="flex items-center gap-2 mb-2">
      <div className="flex-1 h-1 bg-muted/80 rounded-full overflow-hidden">
        <div
          className={`h-full rounded-full transition-all duration-500 ${
            isAdaptive ? 'bg-status-info-text/80' : 'bg-foreground/25'
          }`}
          style={{ width: `${pct}%` }}
        />
      </div>
      <span className="w-8 text-right text-caption tabular-nums text-muted-foreground/75">{pct}%</span>
    </div>
  );
}

function AdaptiveBadge({ isAdaptive }: { isAdaptive: boolean }) {
  if (!isAdaptive) return null;
  return (
    <span className="text-micro px-1.5 py-0.5 rounded border border-[hsl(var(--status-info-text))/0.16] bg-status-info-bg text-status-info-text uppercase tracking-wider">
      AGE
    </span>
  );
}

/** Medium-density card for history grid (no timer, no quick actions) */
export function MissionCardMedium({ mission, onClick }: Pick<MissionCardProps, 'mission' | 'onClick'>) {
  const { t } = useTranslation();
  const { isAdaptive, total, pct } = computeProgress(mission);

  return (
    <div
      onClick={() => onClick(mission.mission_id)}
      className="group p-3 rounded-md border bg-card hover:bg-accent/30 transition-colors cursor-pointer"
    >
      <div className="flex items-center gap-1.5 mb-1.5">
        <StatusBadge status={MISSION_STATUS_MAP[mission.status]}>
          {t(`mission.${mission.status}`)}
        </StatusBadge>
        <AdaptiveBadge isAdaptive={isAdaptive} />
      </div>

      <p className="text-xs font-medium leading-snug line-clamp-2 mb-2">{mission.goal}</p>

      <ProgressBar total={total} pct={pct} isAdaptive={isAdaptive} />

      <div className="flex items-center gap-2 text-caption text-muted-foreground/70">
        <span className="max-w-[96px] truncate">{mission.agent_name}</span>
        {mission.total_tokens_used > 0 && (
          <span className="tabular-nums">{(mission.total_tokens_used / 1000).toFixed(1)}k</span>
        )}
        {mission.attached_doc_count > 0 && <span>📎{mission.attached_doc_count}</span>}
        <span className="ml-auto tabular-nums">{formatDate(mission.updated_at)}</span>
      </div>
    </div>
  );
}

export function MissionCard({ mission, onClick, onQuickStart, onQuickPause }: MissionCardProps) {
  const { t } = useTranslation();
  const isLive = mission.status === 'planning' || mission.status === 'running';
  const canQuickStart = ['draft', 'planned', 'paused', 'failed'].includes(mission.status);
  const canQuickPause = mission.status === 'planning' || mission.status === 'running';

  // Elapsed timer
  const [elapsed, setElapsed] = useState('');
  useEffect(() => {
    if (!isLive) { setElapsed(''); return; }
    const start = new Date(mission.created_at).getTime();
    const tick = () => {
      const sec = Math.round((Date.now() - start) / 1000);
      const m = Math.floor(sec / 60);
      const s = sec % 60;
      setElapsed(`${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`);
    };
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [isLive, mission.created_at]);

  // Health: running > 10 min
  const isLongRunning = isLive && (Date.now() - new Date(mission.created_at).getTime()) > 600_000;

  const { isAdaptive, total, pct } = computeProgress(mission);

  return (
    <div
      onClick={() => onClick(mission.mission_id)}
      className={`group relative p-3.5 rounded-md border bg-card hover:bg-accent/30 transition-colors cursor-pointer ${
        isLongRunning ? 'border-l-[3px] border-l-amber-400' : 'border-l-[3px] border-l-transparent'
      }`}
    >
      {/* Row 1: Status dot + agent + elapsed */}
      <div className="flex items-center gap-2 mb-2">
        <span className={`w-1.5 h-1.5 rounded-full shrink-0 ${statusDot[mission.status]}`} />
        <span className="text-caption text-muted-foreground/70 uppercase tracking-wider font-medium">
          {t(`mission.${mission.status}`)}
        </span>
        <AdaptiveBadge isAdaptive={isAdaptive} />
        <div className="ml-auto flex items-center gap-2 text-caption text-muted-foreground/75">
          {elapsed && <span className="font-mono tabular-nums">{elapsed}</span>}
          <span className="max-w-[96px] truncate">{mission.agent_name}</span>
        </div>
      </div>

      {/* Goal */}
      <p className="text-xs font-medium leading-snug line-clamp-2 mb-2.5">{mission.goal}</p>

      <ProgressBar total={total} pct={pct} isAdaptive={isAdaptive} />

      {/* Footer: last active + tokens + doc count */}
      <div className="flex items-center gap-2 text-caption text-muted-foreground/70">
        <span className="tabular-nums">{formatDate(mission.updated_at)}</span>
        {mission.attached_doc_count > 0 && (
          <span>📎 {mission.attached_doc_count}</span>
        )}
        <span className="ml-auto tabular-nums">{mission.total_tokens_used > 0 ? `${(mission.total_tokens_used / 1000).toFixed(1)}k tok` : ''}</span>
      </div>

      {/* Quick actions: always visible on mobile, hover on desktop */}
      {(canQuickStart || canQuickPause) && (
        <div className="flex sm:hidden sm:group-hover:flex absolute top-2.5 right-2.5 gap-1">
          {canQuickStart && onQuickStart && (
            <button
              onClick={(e) => { e.stopPropagation(); onQuickStart(mission.mission_id); }}
              className="w-7 h-7 sm:w-6 sm:h-6 flex items-center justify-center rounded-md bg-foreground/90 text-background text-xs hover:bg-foreground transition-colors shadow-sm"
            >
              ▶
            </button>
          )}
          {canQuickPause && onQuickPause && (
            <button
              onClick={(e) => { e.stopPropagation(); onQuickPause(mission.mission_id); }}
              className="w-7 h-7 sm:w-6 sm:h-6 flex items-center justify-center rounded-md border border-border bg-background text-xs hover:bg-accent transition-colors shadow-sm"
            >
              ⏸
            </button>
          )}
        </div>
      )}
    </div>
  );
}
