import { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Puzzle } from 'lucide-react';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { Input } from '../ui/input';
import { Switch } from '../ui/switch';
import { FixedExtensionEntry, useConfig } from '../ConfigContext';
import { toggleExtension } from '../settings/extensions/extension-manager';
import { toastService } from '../../toasts';
import { getFriendlyTitle } from '../settings/extensions/subcomponents/ExtensionList';

interface BottomMenuExtensionSelectionProps {
  sessionId: string;
}

export const BottomMenuExtensionSelection = ({ sessionId }: BottomMenuExtensionSelectionProps) => {
  const { t } = useTranslation('common');
  const { t: tExt } = useTranslation('extensions');
  const [searchQuery, setSearchQuery] = useState('');
  const [isOpen, setIsOpen] = useState(false);
  const { extensionsList, addExtension } = useConfig();

  const handleToggle = useCallback(
    async (extensionConfig: FixedExtensionEntry) => {
      if (!sessionId) {
        toastService.error({
          title: t('extensionToggleError'),
          msg: t('noActiveSession'),
          traceback: 'No session ID available',
        });
        return;
      }

      try {
        const toggleDirection = extensionConfig.enabled ? 'toggleOff' : 'toggleOn';

        await toggleExtension({
          toggle: toggleDirection,
          extensionConfig: extensionConfig,
          addToConfig: addExtension,
          toastOptions: { silent: false },
          sessionId: sessionId,
        });
      } catch (error) {
        const action = extensionConfig.enabled ? t('disable').toLowerCase() : t('enable').toLowerCase();
        toastService.error({
          title: t('extensionError'),
          msg: t('failedToToggleExtension', { action, name: extensionConfig.name }),
          traceback: error instanceof Error ? error.message : String(error),
        });
      }
    },
    [sessionId, addExtension, t]
  );

  const filteredExtensions = useMemo(() => {
    return extensionsList.filter((ext) => {
      const query = searchQuery.toLowerCase();
      return (
        ext.name.toLowerCase().includes(query) ||
        (ext.description && ext.description.toLowerCase().includes(query))
      );
    });
  }, [extensionsList, searchQuery]);

  const sortedExtensions = useMemo(() => {
    const getTypePriority = (type: string): number => {
      const priorities: Record<string, number> = {
        builtin: 0,
        platform: 1,
        frontend: 2,
      };
      return priorities[type] ?? Number.MAX_SAFE_INTEGER;
    };

    return [...filteredExtensions].sort((a, b) => {
      // First sort by priority type
      const typeDiff = getTypePriority(a.type) - getTypePriority(b.type);
      if (typeDiff !== 0) return typeDiff;

      // Then sort by enabled status (enabled first)
      if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;

      // Finally sort alphabetically
      return a.name.localeCompare(b.name);
    });
  }, [filteredExtensions]);

  const activeCount = useMemo(() => {
    return extensionsList.filter((ext) => ext.enabled).length;
  }, [extensionsList]);

  return (
    <DropdownMenu
      open={isOpen}
      onOpenChange={(open) => {
        setIsOpen(open);
        if (!open) {
          setSearchQuery(''); // Reset search when closing
        }
      }}
    >
      <DropdownMenuTrigger asChild>
        <button
          className="flex items-center cursor-pointer [&_svg]:size-4 text-text-default/70 hover:text-text-default hover:scale-100 hover:bg-transparent text-xs"
          title={tExt('manageExtensions')}
        >
          <Puzzle className="mr-1 h-4 w-4" />
          <span>{activeCount}</span>
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="top" align="center" className="w-64">
        <div className="p-2">
          <Input
            type="text"
            placeholder={tExt('searchExtensions')}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-8 text-sm"
            autoFocus
          />
        </div>
        <div className="max-h-[400px] overflow-y-auto">
          {sortedExtensions.length === 0 ? (
            <div className="px-2 py-4 text-center text-sm text-text-default/70">
              {searchQuery ? t('noExtensionsFound') : t('noExtensionsAvailable')}
            </div>
          ) : (
            sortedExtensions.map((ext) => (
              <div
                key={ext.name}
                className="flex items-center justify-between px-2 py-2 hover:bg-background-hover cursor-pointer"
                onClick={() => handleToggle(ext)}
                title={ext.description || ext.name}
              >
                <div className="text-sm font-medium text-text-default">{getFriendlyTitle(ext)}</div>
                <div onClick={(e) => e.stopPropagation()}>
                  <Switch
                    checked={ext.enabled}
                    onCheckedChange={() => handleToggle(ext)}
                    variant="mono"
                  />
                </div>
              </div>
            ))
          )}
        </div>
      </DropdownMenuContent>
    </DropdownMenu>
  );
};
