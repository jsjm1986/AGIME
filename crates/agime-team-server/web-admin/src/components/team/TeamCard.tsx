import { useNavigate } from 'react-router-dom';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import type { Team } from '../../api/types';
import { formatDate } from '../../utils/format';

interface TeamCardProps {
  team: Team;
}

export function TeamCard({ team }: TeamCardProps) {
  const navigate = useNavigate();

  return (
    <Card
      className="ui-section-panel group cursor-pointer overflow-hidden transition-[transform,border-color,background-color] hover:-translate-y-0.5 hover:border-[hsl(var(--ui-line-strong))/0.72] hover:bg-[hsl(var(--ui-surface-panel-strong))/0.96]"
      onClick={() => navigate(`/teams/${team.id}`)}
    >
      <CardHeader className="pb-3">
        <div className="ui-kicker">Team</div>
        <CardTitle className="ui-heading text-[22px]">{team.name}</CardTitle>
        {team.description && (
          <p className="ui-secondary-text line-clamp-2 text-sm leading-6">
            {team.description}
          </p>
        )}
      </CardHeader>
      <CardContent>
        <p className="ui-tertiary-text text-xs font-medium tracking-[0.08em] uppercase">
          {formatDate(team.createdAt)}
        </p>
      </CardContent>
    </Card>
  );
}
