import { useState, useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { documentApi } from '../../api/documents';
import type { DocumentSummary, LockInfo } from '../../api/documents';
import { MonacoEditorWrapper } from './editors/MonacoEditor';
import { MarkdownEditor } from './editors/MarkdownEditor';

interface DocumentEditorProps {
  teamId: string;
  document: DocumentSummary;
  initialContent: string;
  lock: LockInfo;
  onSave: () => void;
  onClose: () => void;
}

function formatTimeRemaining(ms: number): string {
  if (ms <= 0) return '0:00';
  const minutes = Math.floor(ms / 60000);
  const seconds = Math.floor((ms % 60000) / 1000);
  return `${minutes}:${seconds.toString().padStart(2, '0')}`;
}

export function DocumentEditor({
  teamId,
  document: doc,
  initialContent,
  lock,
  onSave,
  onClose,
}: DocumentEditorProps) {
  const { t } = useTranslation();
  const [content, setContent] = useState(initialContent);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [message, setMessage] = useState('');
  const [showSaveDialog, setShowSaveDialog] = useState(false);
  const [lockExpired, setLockExpired] = useState(false);
  const [timeRemaining, setTimeRemaining] = useState<number>(0);
  const refreshTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const hasChanges = content !== initialContent;

  // Lock expiry countdown + auto-refresh
  useEffect(() => {
    const updateCountdown = () => {
      const expiresAt = new Date(lock.expires_at).getTime();
      const remaining = expiresAt - Date.now();
      setTimeRemaining(remaining);
      if (remaining <= 0) {
        setLockExpired(true);
      }
    };

    updateCountdown();
    const countdownTimer = setInterval(updateCountdown, 1000);

    // Refresh lock every 10 minutes to prevent expiry during editing
    refreshTimerRef.current = setInterval(async () => {
      try {
        await documentApi.acquireLock(teamId, doc.id);
        setLockExpired(false);
      } catch {
        setLockExpired(true);
      }
    }, 10 * 60 * 1000);

    return () => {
      clearInterval(countdownTimer);
      if (refreshTimerRef.current) clearInterval(refreshTimerRef.current);
    };
  }, [teamId, doc.id, lock.expires_at]);

  const handleSave = useCallback(async () => {
    if (!hasChanges || lockExpired) return;
    setSaveError(null);
    setShowSaveDialog(true);
  }, [hasChanges, lockExpired]);

  const confirmSave = useCallback(async () => {
    setSaving(true);
    setSaveError(null);
    try {
      await documentApi.updateContent(
        teamId,
        doc.id,
        content,
        message || t('documents.defaultUpdateMessage'),
      );
      // Release lock after successful save
      try {
        await documentApi.releaseLock(teamId, doc.id);
      } catch {
        // ignore release errors
      }
      setShowSaveDialog(false);
      setMessage('');
      onSave();
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  }, [teamId, doc.id, content, message, onSave]);

  const handleDiscard = useCallback(async () => {
    try {
      await documentApi.releaseLock(teamId, doc.id);
    } catch {
      // ignore
    }
    onClose();
  }, [teamId, doc.id, onClose]);

  const isMarkdown = doc.mime_type === 'text/markdown';
  const isLockWarning = timeRemaining > 0 && timeRemaining < 5 * 60 * 1000;

  return (
    <div className="flex flex-col h-full border-l">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-4 py-2 border-b bg-muted/30">
        <div className="flex items-center gap-2 flex-1 min-w-0">
          <span className="font-medium truncate text-sm">
            {doc.display_name || doc.name}
          </span>
          <span className="text-xs text-muted-foreground px-2 py-0.5 bg-green-100 dark:bg-green-900 rounded">
            {t('documents.edit')}
          </span>
          {hasChanges && (
            <span className="text-xs text-orange-600 dark:text-orange-400">*</span>
          )}
        </div>
        <div className="flex items-center gap-1 flex-shrink-0">
          {/* Lock countdown */}
          <span className={`text-xs px-2 py-0.5 rounded ${
            lockExpired
              ? 'bg-red-100 dark:bg-red-900 text-red-700 dark:text-red-300'
              : isLockWarning
              ? 'bg-orange-100 dark:bg-orange-900 text-orange-700 dark:text-orange-300'
              : 'text-muted-foreground'
          }`}>
            {lockExpired ? t('documents.lockExpired') : formatTimeRemaining(timeRemaining)}
          </span>
          <Button
            size="sm"
            onClick={handleSave}
            disabled={!hasChanges || saving || lockExpired}
          >
            {saving ? t('documents.saving') : t('documents.save')}
          </Button>
          <Button size="sm" variant="ghost" onClick={handleDiscard}>
            {t('documents.discard')}
          </Button>
        </div>
      </div>

      {/* Lock expired banner */}
      {lockExpired && (
        <div className="px-4 py-2 bg-red-50 dark:bg-red-950 text-red-700 dark:text-red-300 text-xs border-b">
          {t('documents.lockExpiredMessage')}
        </div>
      )}

      {/* Save error banner */}
      {saveError && (
        <div className="px-4 py-2 bg-red-50 dark:bg-red-950 text-red-700 dark:text-red-300 text-xs border-b">
          {saveError}
        </div>
      )}

      {/* Editor */}
      <div className="flex-1 overflow-hidden">
        {isMarkdown ? (
          <MarkdownEditor value={content} onChange={setContent} />
        ) : (
          <MonacoEditorWrapper
            value={content}
            onChange={setContent}
            fileName={doc.name}
          />
        )}
      </div>

      {/* Save dialog */}
      {showSaveDialog && (
        <div className="absolute inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-background rounded-lg p-6 w-96 shadow-lg">
            <h3 className="font-medium mb-3">{t('documents.changeMessage')}</h3>
            <Input
              value={message}
              onChange={(e) => setMessage(e.target.value)}
              placeholder={t('documents.changeMessagePlaceholder')}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === 'Enter') confirmSave();
              }}
            />
            <div className="flex justify-end gap-2 mt-4">
              <Button
                variant="outline"
                size="sm"
                onClick={() => setShowSaveDialog(false)}
              >
                {t('common.cancel')}
              </Button>
              <Button size="sm" onClick={confirmSave} disabled={saving}>
                {saving ? t('documents.saving') : t('documents.save')}
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
