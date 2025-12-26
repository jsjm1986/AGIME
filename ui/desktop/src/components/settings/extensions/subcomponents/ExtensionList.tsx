import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import ExtensionItem from './ExtensionItem';
import builtInExtensionsData from '../../../../built-in-extensions.json';
import { combineCmdAndArgs } from '../utils';
import { ExtensionConfig } from '../../../../api';
import { FixedExtensionEntry } from '../../../ConfigContext';

interface ExtensionListProps {
  extensions: FixedExtensionEntry[];
  onToggle: (extension: FixedExtensionEntry) => Promise<boolean | void> | void;
  onConfigure?: (extension: FixedExtensionEntry) => void;
  isStatic?: boolean;
  disableConfiguration?: boolean;
  searchTerm?: string;
  pendingActivationExtensions?: Set<string>;
}

export default function ExtensionList({
  extensions,
  onToggle,
  onConfigure,
  isStatic,
  disableConfiguration: _disableConfiguration,
  searchTerm = '',
  pendingActivationExtensions = new Set(),
}: ExtensionListProps) {
  const { t } = useTranslation('extensions');
  const matchesSearch = (extension: FixedExtensionEntry): boolean => {
    if (!searchTerm) return true;

    const searchLower = searchTerm.toLowerCase();
    const title = getFriendlyTitle(extension).toLowerCase();
    const name = extension.name.toLowerCase();
    const subtitle = getSubtitle(extension, t);
    const description = subtitle.description?.toLowerCase() || '';

    return (
      title.includes(searchLower) || name.includes(searchLower) || description.includes(searchLower)
    );
  };

  // Separate enabled and disabled extensions, then filter by search term
  const enabledExtensions = extensions.filter((ext) => ext.enabled && matchesSearch(ext));
  const disabledExtensions = extensions.filter((ext) => !ext.enabled && matchesSearch(ext));

  // Sort each group alphabetically by their friendly title
  const sortedEnabledExtensions = [...enabledExtensions].sort((a, b) =>
    getFriendlyTitle(a).localeCompare(getFriendlyTitle(b))
  );
  const sortedDisabledExtensions = [...disabledExtensions].sort((a, b) =>
    getFriendlyTitle(a).localeCompare(getFriendlyTitle(b))
  );

  return (
    <div className="space-y-8">
      {sortedEnabledExtensions.length > 0 && (
        <div>
          <h2 className="text-lg font-medium text-text-default mb-4 flex items-center gap-2">
            <span className="w-2 h-2 bg-green-500 rounded-full"></span>
            {t('enabledExtensionsCount', { count: sortedEnabledExtensions.length })}
          </h2>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 gap-2">
            {sortedEnabledExtensions.map((extension) => (
              <ExtensionItem
                key={extension.name}
                extension={extension}
                onToggle={onToggle}
                onConfigure={onConfigure}
                isStatic={isStatic}
                isPendingActivation={pendingActivationExtensions.has(extension.name)}
              />
            ))}
          </div>
        </div>
      )}

      {sortedDisabledExtensions.length > 0 && (
        <div>
          <h2 className="text-lg font-medium text-text-muted mb-4 flex items-center gap-2">
            <span className="w-2 h-2 bg-gray-400 rounded-full"></span>
            {t('availableExtensions')} ({sortedDisabledExtensions.length})
          </h2>
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5 gap-2">
            {sortedDisabledExtensions.map((extension) => (
              <ExtensionItem
                key={extension.name}
                extension={extension}
                onToggle={onToggle}
                onConfigure={onConfigure}
                isStatic={isStatic}
              />
            ))}
          </div>
        </div>
      )}

      {extensions.length === 0 && (
        <div className="text-center text-text-muted py-8">{t('noExtensionsAvailable')}</div>
      )}
    </div>
  );
}

// Helper functions
export function getFriendlyTitle(extension: FixedExtensionEntry): string {
  const name = (extension.type === 'builtin' && extension.display_name) || extension.name;
  return name
    .split(/[-_]/) // Split on hyphens and underscores
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

function normalizeExtensionName(name: string): string {
  return name
    .toLowerCase()
    .replace(/\s+/g, '-')     // Replace spaces with hyphens
    .replace(/[()]/g, '-')    // Replace parentheses with hyphens
    .replace(/-+/g, '-')      // Collapse multiple hyphens into one
    .replace(/^-|-$/g, '');   // Remove leading/trailing hyphens
}

export function getSubtitle(config: ExtensionConfig, t?: TFunction) {
  switch (config.type) {
    case 'builtin': {
      const extensionData = builtInExtensionsData.find(
        (ext) => normalizeExtensionName(ext.name) === normalizeExtensionName(config.name)
      );
      // Try to get translated description first
      const extensionKey = normalizeExtensionName(config.name);
      const translatedDesc = t ? t(`builtin.${extensionKey}`, { defaultValue: '' }) : '';
      const fallbackDesc = extensionData?.description || config.description || (t ? t('builtInExtension') : 'Built-in extension');
      return {
        description: translatedDesc || fallbackDesc,
        command: null,
      };
    }
    case 'sse':
    case 'streamable_http': {
      const prefixKey = config.type === 'sse' ? 'sseExtension' : 'streamableHttpExtension';
      const prefix = t ? t(prefixKey) : `${config.type.toUpperCase().replace('_', ' ')} extension`;
      return {
        description: `${prefix}${config.description ? ': ' + config.description : ''}`,
        command: config.uri || null,
      };
    }

    default: {
      // Try to get translated description for any extension type
      const extensionKey = normalizeExtensionName(config.name);
      const translatedDesc = t ? t(`builtin.${extensionKey}`, { defaultValue: '' }) : '';
      return {
        description: translatedDesc || config.description || null,
        command: 'cmd' in config ? combineCmdAndArgs(config.cmd, config.args) : null,
      };
    }
  }
}
