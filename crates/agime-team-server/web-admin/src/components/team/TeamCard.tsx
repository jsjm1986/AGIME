import { useNavigate } from 'react-router-dom';
import { Card, CardContent, CardHeader, CardTitle } from '../ui/card';
import type { Team } from '../../api/types';

interface TeamCardProps {
  team: Team;
}

export function TeamCard({ team }: TeamCardProps) {
  const navigate = useNavigate();

  return (
    <Card
      className="cursor-pointer hover:shadow-md transition-shadow"
      onClick={() => navigate(`/teams/${team.id}`)}
    >
      <CardHeader className="pb-2">
        <CardTitle className="text-lg">{team.name}</CardTitle>
        {team.description && (
          <p className="text-sm text-[hsl(var(--muted-foreground))] line-clamp-2">
            {team.description}
          </p>
        )}
      </CardHeader>
      <CardContent>
        <p className="text-xs text-[hsl(var(--muted-foreground))]">
          {new Date(team.createdAt).toLocaleDateString()}
        </p>
      </CardContent>
    </Card>
  );
}
