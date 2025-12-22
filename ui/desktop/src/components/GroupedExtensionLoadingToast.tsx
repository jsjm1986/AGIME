import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from './ui/collapsible';
import { ChevronDown, ChevronUp, Loader2, Check, X, AlertCircle, Puzzle } from 'lucide-react';
import { Button } from './ui/button';
import { startNewSession } from '../sessions';
import { useNavigation } from '../hooks/useNavigation';
import { formatExtensionErrorMessage } from '../utils/extensionErrorUtils';
import { cn } from '../utils';

export interface ExtensionLoadingStatus {
  name: string;
  status: 'loading' | 'success' | 'error';
  error?: string;
  recoverHints?: string;
}

interface ExtensionLoadingToastProps {
  extensions: ExtensionLoadingStatus[];
  totalCount: number;
  isComplete: boolean;
}

export function GroupedExtensionLoadingToast({
  extensions,
  totalCount,
  isComplete,
}: ExtensionLoadingToastProps) {
  const { t } = useTranslation('extensions');
  const [isOpen, setIsOpen] = useState(false);
  const [copiedExtension, setCopiedExtension] = useState<string | null>(null);
  const setView = useNavigation();

  const successCount = extensions.filter((ext) => ext.status === 'success').length;
  const errorCount = extensions.filter((ext) => ext.status === 'error').length;
  const loadingCount = extensions.filter((ext) => ext.status === 'loading').length;

  const getStatusIcon = (status: 'loading' | 'success' | 'error') => {
    switch (status) {
      case 'loading':
        return (
          <div className="w-5 h-5 rounded-full bg-cyan-500/20 flex items-center justify-center">
            <Loader2 className="w-3 h-3 animate-spin text-cyan-400" />
          </div>
        );
      case 'success':
        return (
          <div className="w-5 h-5 rounded-full bg-emerald-500/25 flex items-center justify-center">
            <Check className="w-3 h-3 text-emerald-400" />
          </div>
        );
      case 'error':
        return (
          <div className="w-5 h-5 rounded-full bg-red-500/20 flex items-center justify-center">
            <X className="w-3 h-3 text-red-400" />
          </div>
        );
    }
  };

  const getSummaryText = () => {
    if (!isComplete) {
      return t('loading.loadingExtensions', { count: totalCount });
    }

    if (errorCount === 0) {
      return t('loading.successfullyLoaded', { count: successCount });
    }

    return t('loading.loadedPartial', { success: successCount, total: totalCount });
  };

  const getSummaryIcon = () => {
    if (!isComplete) {
      return (
        <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-cyan-500/30 to-blue-500/30 flex items-center justify-center border border-cyan-500/30 shadow-lg shadow-cyan-500/10">
          <Loader2 className="w-5 h-5 animate-spin text-cyan-400" />
        </div>
      );
    }

    if (errorCount === 0) {
      return (
        <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-emerald-500/30 to-teal-500/30 flex items-center justify-center border border-emerald-500/30 shadow-lg shadow-emerald-500/10">
          <Puzzle className="w-5 h-5 text-emerald-400" />
        </div>
      );
    }

    return (
      <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-amber-500/30 to-orange-500/30 flex items-center justify-center border border-amber-500/30 shadow-lg shadow-amber-500/10">
        <AlertCircle className="w-5 h-5 text-amber-400" />
      </div>
    );
  };

  return (
    <div className="w-full min-w-[280px]">
      <Collapsible open={isOpen} onOpenChange={setIsOpen}>
        <div className="flex flex-col">
          {/* Main summary section - clickable */}
          <CollapsibleTrigger asChild>
            <div className="flex items-center gap-3 cursor-pointer hover:opacity-90 transition-opacity group">
              {getSummaryIcon()}
              <div className="flex-1 min-w-0">
                <div className="font-medium text-sm text-text-default dark:text-white">{getSummaryText()}</div>
                {!isComplete && loadingCount > 0 && (
                  <div className="text-xs text-text-muted dark:text-white/50 mt-0.5 flex items-center gap-1.5">
                    <span className="w-1 h-1 rounded-full bg-cyan-400 animate-pulse" />
                    {loadingCount} {t('loading.inProgress', 'loading...')}
                  </div>
                )}
                {isComplete && errorCount > 0 && (
                  <div className="text-xs text-amber-400/90 mt-0.5">
                    {t('loading.failedToLoad', { count: errorCount })}
                  </div>
                )}
              </div>
              {/* Expand indicator */}
              <div className={cn(
                "w-7 h-7 rounded-lg flex items-center justify-center transition-all",
                "bg-black/5 group-hover:bg-black/10 border border-black/5 dark:bg-white/5 dark:group-hover:bg-white/10 dark:border-white/5",
                isOpen && "bg-black/10 border-black/10 dark:bg-white/10 dark:border-white/10"
              )}>
                {isOpen ? (
                  <ChevronUp className="w-4 h-4 text-text-muted dark:text-white/60" />
                ) : (
                  <ChevronDown className="w-4 h-4 text-text-muted dark:text-white/60" />
                )}
              </div>
            </div>
          </CollapsibleTrigger>

          {/* Expanded details section */}
          <CollapsibleContent className="overflow-hidden data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0">
            <div className="mt-3 pt-3 border-t border-black/10 dark:border-white/10">
              {/* Stats bar */}
              <div className="flex items-center gap-4 mb-3 text-xs">
                <div className="flex items-center gap-1.5">
                  <div className="w-2 h-2 rounded-full bg-emerald-400 shadow-sm shadow-emerald-400/50" />
                  <span className="text-text-muted dark:text-white/60">{successCount} {t('loading.success', 'success')}</span>
                </div>
                {errorCount > 0 && (
                  <div className="flex items-center gap-1.5">
                    <div className="w-2 h-2 rounded-full bg-red-400 shadow-sm shadow-red-400/50" />
                    <span className="text-text-muted dark:text-white/60">{errorCount} {t('loading.failed', 'failed')}</span>
                  </div>
                )}
                {loadingCount > 0 && (
                  <div className="flex items-center gap-1.5">
                    <div className="w-2 h-2 rounded-full bg-cyan-400 animate-pulse shadow-sm shadow-cyan-400/50" />
                    <span className="text-text-muted dark:text-white/60">{loadingCount} {t('loading.pending', 'pending')}</span>
                  </div>
                )}
              </div>

              {/* Extension list */}
              <div className="space-y-1.5 max-h-56 overflow-y-auto pr-1 scrollbar-thin">
                {extensions.map((ext, index) => (
                  <div
                    key={ext.name}
                    className={cn(
                      "flex flex-col gap-1.5 p-2.5 rounded-lg transition-all duration-200",
                      ext.status === 'error'
                        ? "bg-red-500/10 border border-red-500/20"
                        : "bg-black/5 hover:bg-black/10 dark:bg-white/5 dark:hover:bg-white/8 border border-transparent"
                    )}
                    style={{ animationDelay: `${index * 30}ms` }}
                  >
                    <div className="flex items-center gap-2.5">
                      {getStatusIcon(ext.status)}
                      <span className={cn(
                        "flex-1 text-sm font-medium truncate",
                        ext.status === 'success' && "text-text-default dark:text-white/90",
                        ext.status === 'loading' && "text-text-muted dark:text-white/60",
                        ext.status === 'error' && "text-red-600 dark:text-red-300"
                      )}>
                        {ext.name}
                      </span>
                    </div>

                    {ext.status === 'error' && ext.error && (
                      <div className="ml-7 flex flex-col gap-2">
                        <div className="text-xs text-red-600/80 dark:text-red-300/70 break-words leading-relaxed">
                          {formatExtensionErrorMessage(ext.error, 'Failed to add extension')}
                        </div>
                        {ext.recoverHints && setView ? (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={(e) => {
                              e.stopPropagation();
                              startNewSession(ext.recoverHints, setView);
                            }}
                            className="self-start h-7 text-xs bg-black/5 border-black/10 hover:bg-black/10 hover:border-black/20 text-text-default dark:bg-white/5 dark:border-white/10 dark:hover:bg-white/10 dark:hover:border-white/20 dark:text-white/80"
                          >
                            {t('loading.askAgime')}
                          </Button>
                        ) : (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={(e) => {
                              e.stopPropagation();
                              navigator.clipboard.writeText(ext.error!);
                              setCopiedExtension(ext.name);
                              setTimeout(() => setCopiedExtension(null), 2000);
                            }}
                            className="self-start h-7 text-xs bg-black/5 border-black/10 hover:bg-black/10 hover:border-black/20 text-text-default dark:bg-white/5 dark:border-white/10 dark:hover:bg-white/10 dark:hover:border-white/20 dark:text-white/80"
                          >
                            {copiedExtension === ext.name ? (
                              <span className="flex items-center gap-1">
                                <Check className="w-3 h-3 text-emerald-400" />
                                {t('loading.copied')}
                              </span>
                            ) : (
                              t('loading.copyError')
                            )}
                          </Button>
                        )}
                      </div>
                    )}
                  </div>
                ))}
              </div>
            </div>
          </CollapsibleContent>
        </div>
      </Collapsible>
    </div>
  );
}
