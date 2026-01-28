// Source Filter Component
// Allows filtering resources by data source

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { DataSource } from './sources/types';

interface SourceFilterProps {
  sources: DataSource[];
  selectedSources: string[] | 'all';
  onSelectionChange: (selection: string[] | 'all') => void;
  className?: string;
}

export const SourceFilter: React.FC<SourceFilterProps> = ({
  sources,
  selectedSources,
  onSelectionChange,
  className = '',
}) => {
  const { t } = useTranslation();

  const isAllSelected = selectedSources === 'all';

  const handleToggleAll = () => {
    if (isAllSelected) {
      // Deselect all - select only local
      onSelectionChange(['local']);
    } else {
      onSelectionChange('all');
    }
  };

  const handleToggleSource = (sourceId: string) => {
    if (isAllSelected) {
      // Switch from 'all' to specific selection excluding this source
      const newSelection = sources
        .map(s => s.id)
        .filter(id => id !== sourceId);
      onSelectionChange(newSelection);
    } else {
      const currentSelection = selectedSources as string[];
      if (currentSelection.includes(sourceId)) {
        // Remove source (but keep at least one)
        const newSelection = currentSelection.filter(id => id !== sourceId);
        if (newSelection.length > 0) {
          onSelectionChange(newSelection);
        }
      } else {
        // Add source
        const newSelection = [...currentSelection, sourceId];
        // If all sources are selected, switch to 'all'
        if (newSelection.length === sources.length) {
          onSelectionChange('all');
        } else {
          onSelectionChange(newSelection);
        }
      }
    }
  };

  const isSourceSelected = (sourceId: string): boolean => {
    if (isAllSelected) return true;
    return (selectedSources as string[]).includes(sourceId);
  };

  const getSourceIcon = (type: DataSource['type']) => {
    switch (type) {
      case 'local':
        return (
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z" />
          </svg>
        );
      case 'cloud':
        return (
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M3 15a4 4 0 004 4h9a5 5 0 10-.1-9.999 5.002 5.002 0 10-9.78 2.096A4.001 4.001 0 003 15z" />
          </svg>
        );
      case 'lan':
        return (
          <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
              d="M8.111 16.404a5.5 5.5 0 017.778 0M12 20h.01m-7.08-7.071c3.904-3.905 10.236-3.905 14.141 0M1.394 9.393c5.857-5.857 15.355-5.857 21.213 0" />
          </svg>
        );
    }
  };

  const getStatusColor = (status: DataSource['status']) => {
    switch (status) {
      case 'online':
        return 'bg-green-500';
      case 'connecting':
        return 'bg-yellow-500';
      case 'offline':
      case 'error':
        return 'bg-red-500';
    }
  };

  return (
    <div className={`flex flex-wrap items-center gap-2 ${className}`}>
      <span className="text-sm text-gray-500 dark:text-gray-400">
        {t('team.sources.filterBy', 'Sources:')}
      </span>

      {/* All sources button */}
      <button
        onClick={handleToggleAll}
        className={`
          px-3 py-1.5 text-sm rounded-full border transition-colors
          ${isAllSelected
            ? 'bg-blue-100 border-blue-300 text-blue-700 dark:bg-blue-900 dark:border-blue-700 dark:text-blue-300'
            : 'bg-gray-100 border-gray-300 text-gray-600 dark:bg-gray-800 dark:border-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700'
          }
        `}
      >
        {t('team.sources.all', 'All')}
      </button>

      {/* Individual source buttons */}
      {sources.map(source => (
        <button
          key={source.id}
          onClick={() => handleToggleSource(source.id)}
          className={`
            flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-full border transition-colors
            ${isSourceSelected(source.id)
              ? 'bg-blue-100 border-blue-300 text-blue-700 dark:bg-blue-900 dark:border-blue-700 dark:text-blue-300'
              : 'bg-gray-100 border-gray-300 text-gray-600 dark:bg-gray-800 dark:border-gray-600 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700'
            }
          `}
          title={source.lastError || undefined}
        >
          {getSourceIcon(source.type)}
          <span>{source.name}</span>
          <span className={`w-2 h-2 rounded-full ${getStatusColor(source.status)}`} />
        </button>
      ))}
    </div>
  );
};

export default SourceFilter;
