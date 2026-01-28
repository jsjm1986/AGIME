// StatsOverview - Statistics overview component
import React from 'react';
import type { DataSource } from '../sources/types';

interface StatsOverviewProps {
  sources: DataSource[];
  totalTeams?: number;
  totalSkills?: number;
  totalRecipes?: number;
  totalExtensions?: number;
}

export const StatsOverview: React.FC<StatsOverviewProps> = ({
  sources,
  totalTeams = 0,
  totalSkills = 0,
  totalRecipes = 0,
  totalExtensions = 0,
}) => {
  const onlineSources = sources.filter(s => s.status === 'online').length;
  const totalSources = sources.length;

  const stats = [
    { label: 'Sources', value: `${onlineSources}/${totalSources}`, icon: '🔌' },
    { label: 'Teams', value: totalTeams, icon: '👥' },
    { label: 'Skills', value: totalSkills, icon: '⚡' },
    { label: 'Recipes', value: totalRecipes, icon: '📋' },
    { label: 'Extensions', value: totalExtensions, icon: '🧩' },
  ];

  return (
    <div className="grid grid-cols-5 gap-4">
      {stats.map(stat => (
        <div
          key={stat.label}
          className="bg-white border rounded-lg p-4 text-center"
        >
          <div className="text-2xl mb-1">{stat.icon}</div>
          <div className="text-2xl font-bold">{stat.value}</div>
          <div className="text-xs text-gray-500">{stat.label}</div>
        </div>
      ))}
    </div>
  );
};
