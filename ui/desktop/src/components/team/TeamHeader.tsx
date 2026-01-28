import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  ArrowLeft,
  Settings,
  LogOut,
  Users,
  Sparkles,
  Book,
  Puzzle,
  RefreshCw,
} from 'lucide-react';
import { Button } from '../ui/button';
import { TeamSummary } from './types';

interface StatPillProps {
  icon: React.ElementType;
  count: number;
  label: string;
  color: string;
  onClick?: () => void;
  isActive?: boolean;
}

const StatPill: React.FC<StatPillProps> = ({
  icon: Icon,
  count,
  label,
  color,
  onClick,
  isActive
}) => {
  const colorClasses: Record<string, string> = {
    teal: 'bg-teal-500/10 text-teal-600 dark:text-teal-400 hover:bg-teal-500/20',
    purple: 'bg-purple-500/10 text-purple-600 dark:text-purple-400 hover:bg-purple-500/20',
    amber: 'bg-amber-500/10 text-amber-600 dark:text-amber-400 hover:bg-amber-500/20',
    blue: 'bg-blue-500/10 text-blue-600 dark:text-blue-400 hover:bg-blue-500/20',
  };

  const activeClasses: Record<string, string> = {
    teal: 'ring-2 ring-teal-500 bg-teal-500/20',
    purple: 'ring-2 ring-purple-500 bg-purple-500/20',
    amber: 'ring-2 ring-amber-500 bg-amber-500/20',
    blue: 'ring-2 ring-blue-500 bg-blue-500/20',
  };

  return (
    <button
      onClick={onClick}
      className={`
        flex items-center gap-2 px-4 py-2.5 rounded-xl transition-all duration-200
        ${colorClasses[color]}
        ${isActive ? activeClasses[color] : ''}
        ${onClick ? 'cursor-pointer' : 'cursor-default'}
      `}
    >
      <Icon size={18} strokeWidth={2} />
      <span className="text-lg font-semibold">{count}</span>
      <span className="text-sm opacity-80">{label}</span>
    </button>
  );
};

interface TeamHeaderProps {
  teamSummary: TeamSummary;
  onBack: () => void;
  onSettings?: () => void;
  onLeave?: () => void;
  onSync?: () => void;
  isSyncing?: boolean;
  isAdminOrOwner: boolean;
  canLeave: boolean;
  activeFilter?: string;
  onFilterChange?: (filter: string) => void;
}

const TeamHeader: React.FC<TeamHeaderProps> = ({
  teamSummary,
  onBack,
  onSettings,
  onLeave,
  onSync,
  isSyncing,
  isAdminOrOwner,
  canLeave,
  activeFilter,
  onFilterChange,
}) => {
  const { t } = useTranslation('team');
  const { team, membersCount, skillsCount, recipesCount, extensionsCount } = teamSummary;

  const stats = [
    { icon: Users, count: membersCount, label: t('members'), color: 'teal', filter: 'members' },
    { icon: Sparkles, count: skillsCount, label: t('skills'), color: 'purple', filter: 'skills' },
    { icon: Book, count: recipesCount, label: t('recipes'), color: 'amber', filter: 'recipes' },
    { icon: Puzzle, count: extensionsCount, label: t('extensions'), color: 'blue', filter: 'extensions' },
  ];

  return (
    <div className="relative overflow-hidden">
      {/* Gradient background */}
      <div className="absolute inset-0 bg-gradient-to-br from-teal-500/5 via-purple-500/5 to-blue-500/5 dark:from-teal-500/10 dark:via-purple-500/10 dark:to-blue-500/10" />

      {/* Decorative circles */}
      <div className="absolute -top-24 -right-24 w-48 h-48 bg-teal-500/10 rounded-full blur-3xl" />
      <div className="absolute -bottom-24 -left-24 w-48 h-48 bg-purple-500/10 rounded-full blur-3xl" />

      <div className="relative p-6">
        {/* Top bar */}
        <div className="flex items-center justify-between mb-4">
          <div className="flex items-center gap-4">
            <button
              onClick={onBack}
              className="flex items-center gap-2 text-text-muted hover:text-text-default transition-colors group"
            >
              <ArrowLeft size={18} className="group-hover:-translate-x-1 transition-transform" />
              <span className="text-sm font-medium">{t('backToTeams')}</span>
            </button>
          </div>

          <div className="flex items-center gap-2">
            {onSync && (
              <Button
                variant="ghost"
                size="sm"
                onClick={onSync}
                disabled={isSyncing}
                className="text-text-muted hover:text-text-default"
              >
                <RefreshCw size={16} className={isSyncing ? 'animate-spin' : ''} />
              </Button>
            )}
            {isAdminOrOwner && onSettings && (
              <Button
                variant="ghost"
                size="sm"
                onClick={onSettings}
                className="text-text-muted hover:text-text-default"
              >
                <Settings size={16} />
              </Button>
            )}
            {canLeave && onLeave && (
              <Button
                variant="ghost"
                size="sm"
                onClick={onLeave}
                className="text-red-500 hover:text-red-600 hover:bg-red-500/10"
              >
                <LogOut size={16} />
              </Button>
            )}
          </div>
        </div>

        {/* Team info */}
        <div className="mb-6">
          <h1 className="text-2xl font-bold text-text-default mb-2">
            {team.name}
          </h1>
          {team.description && (
            <p className="text-text-muted max-w-2xl line-clamp-2">
              {team.description}
            </p>
          )}
        </div>

        {/* Stats pills */}
        <div className="flex flex-wrap gap-3">
          {stats.map((stat) => (
            <StatPill
              key={stat.filter}
              icon={stat.icon}
              count={stat.count}
              label={stat.label}
              color={stat.color}
              onClick={onFilterChange ? () => onFilterChange(stat.filter) : undefined}
              isActive={activeFilter === stat.filter}
            />
          ))}
        </div>
      </div>
    </div>
  );
};

export default TeamHeader;
