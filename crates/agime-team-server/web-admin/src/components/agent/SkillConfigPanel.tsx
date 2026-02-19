import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { Badge } from '../ui/badge';
import { AgentSkillConfig, agentApi } from '../../api/agent';

interface AvailableSkill {
  id: string;
  name: string;
  description?: string;
  version: string;
}

interface Props {
  agentId: string;
  teamId: string;
  assignedSkills: AgentSkillConfig[];
  onSkillsChange: (skills: AgentSkillConfig[]) => void;
}

export function SkillConfigPanel({
  agentId,
  teamId,
  assignedSkills,
  onSkillsChange,
}: Props) {
  const { t } = useTranslation();
  const [allSkills, setAllSkills] = useState<AvailableSkill[]>([]);
  const [fetching, setFetching] = useState(false);

  useEffect(() => {
    setFetching(true);
    agentApi
      .listAvailableSkills(agentId, teamId)
      .then(setAllSkills)
      .catch((e) => console.error('Failed to load available skills:', e))
      .finally(() => setFetching(false));
  }, [agentId, teamId]);

  const assignedIds = new Set(assignedSkills.map((s) => s.skill_id));
  const available = allSkills.filter((s) => !assignedIds.has(s.id));

  const handleAdd = (skill: AvailableSkill) => {
    onSkillsChange([
      ...assignedSkills,
      {
        skill_id: skill.id,
        name: skill.name,
        description: skill.description,
        enabled: true,
        version: skill.version,
      },
    ]);
  };

  const handleRemove = (skillId: string) => {
    onSkillsChange(assignedSkills.filter((s) => s.skill_id !== skillId));
  };

  const handleToggle = (skillId: string) => {
    onSkillsChange(
      assignedSkills.map((s) =>
        s.skill_id === skillId ? { ...s, enabled: !s.enabled } : s
      )
    );
  };

  return (
    <div className="space-y-4">
      {/* Assigned skills */}
      <div>
        <h4 className="text-sm font-medium mb-2">{t('agent.skills.assignedSkills')}</h4>
        {assignedSkills.length === 0 ? (
          <p className="text-sm text-muted-foreground py-2">
            {t('agent.skills.noSkillsAssigned')}
          </p>
        ) : (
          <div className="space-y-1.5">
            {assignedSkills.map((skill) => (
              <div
                key={skill.skill_id}
                className="flex items-center justify-between p-2 border rounded bg-accent/30"
              >
                <div className="flex items-center gap-2 min-w-0">
                  <Badge
                    variant={skill.enabled ? 'default' : 'outline'}
                    className="cursor-pointer shrink-0"
                    role="switch"
                    aria-checked={skill.enabled}
                    tabIndex={0}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter' || e.key === ' ') {
                        e.preventDefault();
                        handleToggle(skill.skill_id);
                      }
                    }}
                    onClick={() => handleToggle(skill.skill_id)}
                  >
                    {skill.enabled ? '✓' : '○'}
                  </Badge>
                  <span className="text-sm font-medium">{skill.name}</span>
                  {skill.version && (
                    <span className="text-xs text-muted-foreground">v{skill.version}</span>
                  )}
                  {skill.description && (
                    <span className="text-xs text-muted-foreground truncate max-w-[180px]">
                      {skill.description}
                    </span>
                  )}
                </div>
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 w-6 p-0 shrink-0"
                  aria-label={t('common.remove')}
                  onClick={() => handleRemove(skill.skill_id)}
                >
                  ×
                </Button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Available skills */}
      <div>
        <h4 className="text-sm font-medium mb-2">{t('agent.skills.availableSkills')}</h4>
        {fetching ? (
          <p className="text-sm text-muted-foreground py-2">{t('common.loading')}...</p>
        ) : available.length === 0 ? (
          <p className="text-sm text-muted-foreground py-2">
            {t('agent.skills.noAvailableSkills')}
          </p>
        ) : (
          <div className="space-y-1.5 max-h-[240px] overflow-y-auto">
            {available.map((skill) => (
              <div
                key={skill.id}
                className="flex items-center justify-between p-2 border rounded hover:bg-accent/50 transition-colors"
              >
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="text-sm">{skill.name}</span>
                    <span className="text-xs text-muted-foreground">v{skill.version}</span>
                  </div>
                  {skill.description && (
                    <div className="text-xs text-muted-foreground truncate max-w-[280px]">
                      {skill.description}
                    </div>
                  )}
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  className="h-7 shrink-0 ml-2"
                  onClick={() => handleAdd(skill)}
                >
                  + {t('common.add')}
                </Button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
