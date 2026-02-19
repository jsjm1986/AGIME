import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { documentApi, formatFileSize } from '../../api/documents';
import type { VersionSummary } from '../../api/documents';
import { ConfirmDialog } from '../ui/confirm-dialog';

interface VersionTimelineProps {
  teamId: string;
  docId: string;
  canManage: boolean;
  onSelectVersion?: (version: VersionSummary) => void;
  onCompare?: (v1: VersionSummary, v2: VersionSummary) => void;
  onRollback?: () => void;
}

export function VersionTimeline({
  teamId,
  docId,
  canManage,
  onSelectVersion,
  onCompare,
  onRollback,
}: VersionTimelineProps) {
  const { t } = useTranslation();
  const [versions, setVersions] = useState<VersionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [tagInput, setTagInput] = useState<Record<string, string>>({});
  const [rollbackTarget, setRollbackTarget] = useState<string | null>(null);
  const [editingTag, setEditingTag] = useState<string | null>(null);

  const loadVersions = async () => {
    setLoading(true);
    try {
      const res = await documentApi.listVersions(teamId, docId);
      setVersions(res.items);
    } catch (err) {
      console.error('Failed to load versions:', err);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    loadVersions();
  }, [teamId, docId]);

  const handleToggleSelect = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else if (next.size < 2) {
        next.add(id);
      }
      return next;
    });
  };

  const handleCompare = () => {
    if (selected.size !== 2 || !onCompare) return;
    const ids = Array.from(selected);
    const v1 = versions.find((v) => v.id === ids[0]);
    const v2 = versions.find((v) => v.id === ids[1]);
    if (v1 && v2) {
      // Ensure older version is first
      if (v1.version_number > v2.version_number) {
        onCompare(v2, v1);
      } else {
        onCompare(v1, v2);
      }
    }
  };

  const handleRollback = async (versionId: string) => {
    setRollbackTarget(versionId);
  };

  const confirmRollback = async () => {
    if (!rollbackTarget) return;
    try {
      await documentApi.rollbackVersion(teamId, docId, rollbackTarget);
      loadVersions();
      onRollback?.();
    } catch (err) {
      console.error('Rollback failed:', err);
    } finally {
      setRollbackTarget(null);
    }
  };

  const handleSetTag = async (versionId: string) => {
    const tag = tagInput[versionId]?.trim();
    if (!tag) return;
    try {
      await documentApi.tagVersion(teamId, docId, versionId, tag);
      setEditingTag(null);
      setTagInput((prev) => ({ ...prev, [versionId]: '' }));
      loadVersions();
    } catch (err) {
      console.error('Tag failed:', err);
    }
  };

  if (loading) {
    return <div className="p-4 text-muted-foreground">{t('common.loading')}</div>;
  }

  if (versions.length === 0) {
    return (
      <div className="p-4 text-center text-muted-foreground">
        <p>{t('documents.noVersions')}</p>
        <p className="text-xs mt-1">{t('documents.noVersionsHint')}</p>
      </div>
    );
  }

  return (
    <>
    <div className="flex flex-col h-full">
      {/* Compare toolbar */}
      {onCompare && (
        <div className="px-4 py-2 border-b flex items-center gap-2">
          <span className="text-xs text-muted-foreground">
            {versions.length < 2
              ? t('documents.needTwoVersions')
              : t('documents.selectVersions')}
          </span>
          <Button
            size="sm"
            variant="outline"
            disabled={selected.size !== 2}
            onClick={handleCompare}
          >
            {t('documents.compare')}
          </Button>
        </div>
      )}

      {/* Timeline */}
      <div className="flex-1 overflow-auto p-4">
        <div className="relative">
          {/* Vertical line */}
          <div className="absolute left-4 top-0 bottom-0 w-px bg-border" />

          {versions.map((v, i) => (
            <div key={v.id} className="relative pl-10 pb-6">
              {/* Dot */}
              <div
                className={`absolute left-2.5 top-1 w-3 h-3 rounded-full border-2 ${
                  i === 0
                    ? 'bg-primary border-primary'
                    : 'bg-background border-muted-foreground'
                }`}
              />

              <div className="flex items-start justify-between gap-2">
                <div className="flex-1 min-w-0">
                  {/* Version header */}
                  <div className="flex items-center gap-2 flex-wrap">
                    {onCompare && (
                      <input
                        type="checkbox"
                        checked={selected.has(v.id)}
                        onChange={() => handleToggleSelect(v.id)}
                        className="rounded"
                      />
                    )}
                    <span className="font-medium text-sm">
                      {t('documents.versionNumber', { number: v.version_number })}
                    </span>
                    {i === 0 && (
                      <span className="text-xs px-1.5 py-0.5 bg-primary/10 text-primary rounded">
                        {t('documents.currentVersion')}
                      </span>
                    )}
                    {v.tag && (
                      <span className="text-xs px-1.5 py-0.5 bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300 rounded">
                        {v.tag}
                      </span>
                    )}
                  </div>

                  {/* Message */}
                  <p className="text-sm text-muted-foreground mt-0.5">
                    {v.message}
                  </p>

                  {/* Meta */}
                  <p className="text-xs text-muted-foreground mt-1">
                    {v.created_by_name} · {formatFileSize(v.file_size)} ·{' '}
                    {new Date(v.created_at).toLocaleString()}
                  </p>

                  {/* Tag editing */}
                  {editingTag === v.id && (
                    <div className="flex items-center gap-1 mt-2">
                      <Input
                        value={tagInput[v.id] || ''}
                        onChange={(e) =>
                          setTagInput((prev) => ({
                            ...prev,
                            [v.id]: e.target.value,
                          }))
                        }
                        placeholder={t('documents.tagPlaceholder')}
                        className="h-7 w-32 text-xs"
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') handleSetTag(v.id);
                        }}
                      />
                      <Button
                        size="sm"
                        variant="outline"
                        className="h-7 text-xs"
                        onClick={() => handleSetTag(v.id)}
                      >
                        OK
                      </Button>
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-7 text-xs"
                        onClick={() => setEditingTag(null)}
                      >
                        {t('common.cancel')}
                      </Button>
                    </div>
                  )}
                </div>

                {/* Actions */}
                <div className="flex items-center gap-1 flex-shrink-0">
                  {onSelectVersion && (
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7 text-xs"
                      onClick={() => onSelectVersion(v)}
                    >
                      {t('documents.preview')}
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-7 text-xs"
                    onClick={() => setEditingTag(v.id)}
                  >
                    {t('documents.setTag')}
                  </Button>
                  {canManage && i > 0 && (
                    <Button
                      size="sm"
                      variant="ghost"
                      className="h-7 text-xs text-orange-600"
                      onClick={() => handleRollback(v.id)}
                    >
                      {t('documents.rollback')}
                    </Button>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
    <ConfirmDialog
      open={!!rollbackTarget}
      onOpenChange={(open) => { if (!open) setRollbackTarget(null); }}
      title={t('documents.rollback')}
      description={t('documents.rollbackConfirm')}
      variant="destructive"
      onConfirm={confirmRollback}
    />
    </>
  );
}
