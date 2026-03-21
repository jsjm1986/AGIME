import { useEffect, useRef, type ReactNode } from 'react';
import {
  Archive,
  Check,
  ChevronDown,
  File,
  FileCode2,
  FileImage,
  FileSpreadsheet,
  FileText,
  Music4,
  Video,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { cn } from '../../utils';
import { Button } from '../ui/button';

export interface DocumentFileCardAction {
  key: string;
  label: string;
  icon?: ReactNode;
  tone?: 'default' | 'danger';
  onSelect: () => void;
}

interface DocumentFileCardProps {
  name: string;
  mimeType: string;
  metaLabel: string;
  thumbnailUrl?: string | null;
  active?: boolean;
  compact?: boolean;
  selectionMode?: boolean;
  selected?: boolean;
  actionOpen?: boolean;
  onOpen: () => void;
  onToggleActions?: () => void;
  onToggleSelect?: () => void;
  actions?: DocumentFileCardAction[];
  footer?: ReactNode;
  accessory?: ReactNode;
}

function getTypeLabel(name: string, mimeType: string): string {
  const extension = name.split('.').pop()?.trim();
  if (extension && extension.length <= 6) {
    return extension.toUpperCase();
  }
  if (mimeType.startsWith('image/')) return 'IMAGE';
  if (mimeType.startsWith('video/')) return 'VIDEO';
  if (mimeType.startsWith('audio/')) return 'AUDIO';
  if (mimeType.includes('sheet') || mimeType.includes('excel')) return 'SHEET';
  if (mimeType.includes('pdf')) return 'PDF';
  if (mimeType.includes('json')) return 'JSON';
  if (mimeType.includes('html')) return 'HTML';
  if (mimeType.startsWith('text/')) return 'TEXT';
  return 'FILE';
}

function FileGlyph({ mimeType }: { mimeType: string }) {
  const className = 'h-[18px] w-[18px]';
  if (mimeType.startsWith('image/')) {
    return <FileImage className={className} />;
  }
  if (mimeType.startsWith('video/')) {
    return <Video className={className} />;
  }
  if (mimeType.startsWith('audio/')) {
    return <Music4 className={className} />;
  }
  if (mimeType.includes('sheet') || mimeType.includes('excel')) {
    return <FileSpreadsheet className={className} />;
  }
  if (mimeType.includes('json') || mimeType.includes('javascript') || mimeType.includes('typescript')) {
    return <FileCode2 className={className} />;
  }
  if (mimeType.includes('pdf') || mimeType.startsWith('text/')) {
    return <FileText className={className} />;
  }
  if (mimeType.includes('zip') || mimeType.includes('rar') || mimeType.includes('tar')) {
    return <Archive className={className} />;
  }
  return <File className={className} />;
}

export function DocumentFileCard({
  name,
  mimeType,
  metaLabel,
  thumbnailUrl,
  active = false,
  compact = false,
  selectionMode = false,
  selected = false,
  actionOpen = false,
  onOpen,
  onToggleActions,
  onToggleSelect,
  actions = [],
  footer,
  accessory,
}: DocumentFileCardProps) {
  const { t } = useTranslation();
  const typeLabel = getTypeLabel(name, mimeType);
  const hasActions = actions.length > 0 && !selectionMode;
  const cardRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!actionOpen) {
      return;
    }

    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null;
      if (!target) {
        return;
      }
      if (target.closest('[data-doc-action-menu]') || target.closest('[data-doc-action-trigger]')) {
        return;
      }
      onToggleActions?.();
    };

    const handleEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onToggleActions?.();
      }
    };

    window.addEventListener('pointerdown', handlePointerDown);
    window.addEventListener('keydown', handleEscape);
    return () => {
      window.removeEventListener('pointerdown', handlePointerDown);
      window.removeEventListener('keydown', handleEscape);
    };
  }, [actionOpen, onToggleActions]);

  return (
    <div
      ref={cardRef}
      className={cn(
        'group relative rounded-[14px] border border-[hsl(var(--ui-line-soft))] bg-background transition-colors',
        active
          ? 'border-primary/45 bg-[hsl(var(--ui-surface-selected))]'
          : 'hover:border-[hsl(var(--ui-line-soft))] hover:bg-[hsl(var(--ui-surface-panel))]',
      )}
    >
      <div className={cn('flex items-start gap-2.5', compact ? 'px-3 py-2.5' : 'px-4 py-3')}>
        {selectionMode ? (
          <button
            type="button"
            className={cn(
              'mt-1 flex h-5 w-5 shrink-0 items-center justify-center rounded-full border transition-colors',
              selected ? 'border-primary bg-primary text-primary-foreground' : 'border-border bg-background text-transparent',
            )}
            onClick={(event) => {
              event.stopPropagation();
              onToggleSelect?.();
            }}
            aria-label={selected ? t('documents.exitSelectMode', '取消选择') : t('documents.selectMode', '选择')}
          >
            <Check className="h-3.5 w-3.5" />
          </button>
        ) : null}

        <button
          type="button"
          className="flex min-w-0 flex-1 items-start gap-3 text-left"
          onClick={onOpen}
        >
          <div
            className={cn(
              'flex shrink-0 items-center justify-center overflow-hidden rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] text-muted-foreground',
              compact ? 'h-11 w-11' : 'h-12 w-12',
            )}
          >
            {thumbnailUrl ? (
              <img src={thumbnailUrl} alt="" className="h-full w-full object-cover" />
            ) : (
              <FileGlyph mimeType={mimeType} />
            )}
          </div>

          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-start justify-between gap-3">
              <div
                className={cn(
                  'min-w-0 flex-1 font-semibold text-foreground',
                  compact ? 'line-clamp-2 text-[12.5px] leading-[1.3]' : 'line-clamp-2 text-[14px] leading-[1.35]',
                )}
              >
                {name}
              </div>
              <div className="relative shrink-0">
                {accessory}
                {hasActions ? (
                  actionOpen ? (
                    <div
                      data-doc-action-menu
                      className="absolute right-0 top-0 z-20 w-[132px] overflow-hidden rounded-[12px] border border-[hsl(var(--ui-line-soft))] bg-background shadow-[0_12px_28px_rgba(15,23,42,0.12)]"
                    >
                      <div className="flex flex-col p-1.5">
                        {actions.map((action) => (
                          <button
                            key={action.key}
                            type="button"
                            className={cn(
                              'inline-flex h-8 items-center rounded-[10px] px-2.5 text-left text-[11px] font-medium text-muted-foreground transition-colors hover:bg-[hsl(var(--ui-surface-panel))] hover:text-foreground',
                              action.tone === 'danger' && 'text-destructive hover:bg-[hsl(var(--status-error-bg))] hover:text-destructive',
                            )}
                            onClick={(event) => {
                              event.stopPropagation();
                              action.onSelect();
                              onToggleActions?.();
                            }}
                          >
                            {action.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  ) : (
                    <Button
                      data-doc-action-trigger
                      size="sm"
                      variant="ghost"
                      className={cn(
                        'h-8 shrink-0 rounded-full border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-3 text-[10.5px] font-medium text-muted-foreground shadow-none hover:bg-background hover:text-foreground',
                        compact ? 'gap-0.5' : 'gap-1',
                      )}
                      onClick={(event) => {
                        event.stopPropagation();
                        onToggleActions?.();
                      }}
                    >
                      <ChevronDown className="h-3.5 w-3.5" />
                      {t('common.more', '更多')}
                    </Button>
                  )
                ) : null}
              </div>
            </div>

            <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-1.5 text-[10.5px] leading-4 text-muted-foreground">
              <span className="inline-flex shrink-0 items-center rounded-full border border-[hsl(var(--ui-line-soft))] bg-[hsl(var(--ui-surface-panel))] px-1.5 py-0.5 text-[9.5px] font-semibold uppercase tracking-[0.08em] text-muted-foreground/84">
                {typeLabel}
              </span>
              <span>{metaLabel}</span>
            </div>

            {footer ? (
              <div className="mt-2 min-w-0">{footer}</div>
            ) : null}
          </div>
        </button>
      </div>
    </div>
  );
}
