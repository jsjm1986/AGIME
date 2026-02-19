import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { useNavigate } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Dialog, DialogContent } from './dialog';
import { useTeamContext } from '../../contexts/TeamContext';
import { NAV_ITEMS } from '../../config/teamNavConfig';
import { Search } from 'lucide-react';

interface ResultItem {
  id: string;
  label: string;
  group: 'nav' | 'agent' | 'document';
  action: () => void;
}

const GLOBAL_NAV = [
  { path: '/dashboard', labelKey: 'sidebar.dashboard' },
  { path: '/teams', labelKey: 'sidebar.teams' },
  { path: '/api-keys', labelKey: 'sidebar.apiKeys' },
  { path: '/settings', labelKey: 'sidebar.settings' },
];

export function CommandPalette() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const teamCtx = useTeamContext();
  const inputRef = useRef<HTMLInputElement>(null);

  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);

  // Global shortcut: Cmd/Ctrl+K
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        setOpen(prev => !prev);
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);

  // Focus input when opened
  useEffect(() => {
    if (open) {
      setQuery('');
      setActiveIndex(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  // Build results
  const results = useMemo(() => {
    const items: ResultItem[] = [];
    const q = query.toLowerCase().trim();

    // Global navigation
    for (const item of GLOBAL_NAV) {
      const label = t(item.labelKey);
      if (!q || label.toLowerCase().includes(q)) {
        items.push({
          id: `nav-${item.path}`,
          label,
          group: 'nav',
          action: () => { navigate(item.path); setOpen(false); },
        });
      }
    }

    // Team navigation (if in team context)
    if (teamCtx) {
      for (const item of NAV_ITEMS) {
        const label = t(item.labelKey);
        if (!q || label.toLowerCase().includes(q)) {
          items.push({
            id: `team-nav-${item.key}`,
            label: `${teamCtx.team.name} → ${label}`,
            group: 'nav',
            action: () => { teamCtx.onSectionChange(item.key); setOpen(false); },
          });
        }
      }
    }

    // Limit per group
    const grouped: ResultItem[] = [];
    const counts = { nav: 0, agent: 0, document: 0 };
    for (const item of items) {
      if (counts[item.group] < 5) {
        grouped.push(item);
        counts[item.group]++;
      }
    }
    return grouped;
  }, [query, t, navigate, teamCtx]);

  // Reset active index when results change
  useEffect(() => {
    setActiveIndex(0);
  }, [results.length]);

  // Keyboard navigation
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIndex(i => Math.min(i + 1, results.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIndex(i => Math.max(i - 1, 0));
    } else if (e.key === 'Enter' && results[activeIndex]) {
      e.preventDefault();
      results[activeIndex].action();
    }
  }, [results, activeIndex]);

  const groupLabels: Record<string, string> = {
    nav: t('commandPalette.navigation', 'Navigation'),
    agent: t('commandPalette.agents', 'Agents'),
    document: t('commandPalette.documents', 'Documents'),
  };

  // Group results for display
  const groupedResults = useMemo(() => {
    const groups: { key: string; label: string; items: (ResultItem & { globalIdx: number })[] }[] = [];
    let globalIdx = 0;
    const seen = new Set<string>();
    for (const item of results) {
      if (!seen.has(item.group)) {
        seen.add(item.group);
        groups.push({ key: item.group, label: groupLabels[item.group] || item.group, items: [] });
      }
      const group = groups.find(g => g.key === item.group)!;
      group.items.push({ ...item, globalIdx });
      globalIdx++;
    }
    return groups;
  }, [results, groupLabels]);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent className="p-0 gap-0 max-w-lg" onKeyDown={handleKeyDown}>
        <div className="flex items-center gap-2 px-3 border-b">
          <Search className="w-4 h-4 text-muted-foreground shrink-0" />
          <input
            ref={inputRef}
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder={t('commandPalette.placeholder', 'Search...')}
            className="flex-1 py-3 text-sm bg-transparent outline-none placeholder:text-muted-foreground"
          />
          <kbd className="text-[10px] text-muted-foreground border rounded px-1.5 py-0.5">ESC</kbd>
        </div>
        <div className="max-h-72 overflow-y-auto p-1">
          {results.length === 0 ? (
            <div className="py-6 text-center text-sm text-muted-foreground">
              {t('commandPalette.noResults', 'No results found')}
            </div>
          ) : (
            groupedResults.map(group => (
              <div key={group.key}>
                <div className="px-2 py-1.5 text-[11px] font-medium text-muted-foreground">
                  {group.label}
                </div>
                {group.items.map(item => (
                  <button
                    key={item.id}
                    onClick={item.action}
                    className={`w-full text-left px-2 py-1.5 text-sm rounded-md transition-colors ${
                      item.globalIdx === activeIndex
                        ? 'bg-accent text-accent-foreground'
                        : 'hover:bg-muted'
                    }`}
                    onMouseEnter={() => setActiveIndex(item.globalIdx)}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            ))
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
