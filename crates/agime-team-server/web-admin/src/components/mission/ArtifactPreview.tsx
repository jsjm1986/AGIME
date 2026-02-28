import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

import type { MissionArtifact } from '../../api/mission';
import { SharedPreviewContent } from '../documents/DocumentPreview';

interface ArtifactPreviewProps {
  artifact: MissionArtifact;
  downloadUrl: string;
}

const PREVIEW_COMPACT_HEIGHT = 'h-[360px]';
const PREVIEW_EXPANDED_HEIGHT = 'h-[72vh] min-h-[420px] max-h-[900px]';

function extOf(artifact: MissionArtifact): string {
  const source = artifact.name || artifact.file_path || '';
  const idx = source.lastIndexOf('.');
  if (idx < 0 || idx === source.length - 1) return '';
  return source.slice(idx + 1).toLowerCase();
}

function fallbackMime(ext: string, artifactType: string): string {
  switch (ext) {
    case 'md':
      return 'text/markdown';
    case 'txt':
      return 'text/plain';
    case 'html':
    case 'htm':
      return 'text/html';
    case 'json':
      return 'application/json';
    case 'csv':
    case 'tsv':
      return 'text/csv';
    case 'svg':
      return 'image/svg+xml';
    case 'pdf':
      return 'application/pdf';
    case 'doc':
      return 'application/msword';
    case 'docx':
      return 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';
    case 'xls':
      return 'application/vnd.ms-excel';
    case 'xlsx':
      return 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet';
    case 'ppt':
      return 'application/vnd.ms-powerpoint';
    case 'pptx':
      return 'application/vnd.openxmlformats-officedocument.presentationml.presentation';
    case 'png':
      return 'image/png';
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg';
    case 'gif':
      return 'image/gif';
    case 'bmp':
      return 'image/bmp';
    case 'webp':
      return 'image/webp';
    case 'ico':
      return 'image/x-icon';
    case 'avif':
      return 'image/avif';
    case 'mp3':
      return 'audio/mpeg';
    case 'wav':
      return 'audio/wav';
    case 'ogg':
      return 'audio/ogg';
    case 'm4a':
      return 'audio/mp4';
    case 'mp4':
      return 'video/mp4';
    case 'mov':
      return 'video/quicktime';
    case 'webm':
      return 'video/webm';
    case 'woff':
      return 'font/woff';
    case 'woff2':
      return 'font/woff2';
    case 'ttf':
      return 'font/ttf';
    case 'otf':
      return 'font/otf';
    default:
      if (artifactType === 'code' || artifactType === 'config') return 'text/plain';
      return 'application/octet-stream';
  }
}

function normalizeMime(artifact: MissionArtifact): string {
  const raw = (artifact.mime_type || '').trim().toLowerCase();
  if (raw && raw !== 'application/octet-stream') return raw;
  return fallbackMime(extOf(artifact), (artifact.artifact_type || '').toLowerCase());
}

function previewTitle(mime: string, t: (key: string) => string): string {
  if (mime.includes('json')) return 'JSON';
  if (mime.includes('csv')) return 'CSV';
  if (mime.includes('markdown')) return 'Markdown';
  if (mime.includes('html')) return 'HTML';
  if (mime.includes('svg')) return 'SVG';
  if (mime.includes('pdf')) return 'PDF';
  if (mime.startsWith('image/')) return t('documents.filterImages') || 'Images';
  if (mime.startsWith('audio/') || mime.startsWith('video/')) return t('documents.filterMedia') || 'Media';
  return t('documents.preview') || 'Preview';
}

function shouldUseBlobPreviewUrl(mime: string): boolean {
  return (
    mime === 'application/pdf' ||
    mime.startsWith('image/') ||
    mime.startsWith('audio/') ||
    mime.startsWith('video/')
  );
}

export function ArtifactPreview({ artifact, downloadUrl }: ArtifactPreviewProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const mime = useMemo(() => normalizeMime(artifact), [artifact]);
  const title = useMemo(() => previewTitle(mime, t), [mime, t]);
  const [previewUrl, setPreviewUrl] = useState(downloadUrl);
  const [previewUrlLoading, setPreviewUrlLoading] = useState(false);
  const [previewUrlError, setPreviewUrlError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let objectUrl: string | null = null;
    const useBlob = shouldUseBlobPreviewUrl(mime);

    setPreviewUrl(downloadUrl);
    setPreviewUrlError(null);
    setPreviewUrlLoading(false);

    if (!useBlob) {
      return () => {
        cancelled = true;
      };
    }

    setPreviewUrlLoading(true);
    fetch(downloadUrl, { credentials: 'include' })
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.arrayBuffer();
      })
      .then((buffer) => {
        if (cancelled) return;
        objectUrl = URL.createObjectURL(
          new Blob([buffer], { type: mime || 'application/octet-stream' }),
        );
        setPreviewUrl(objectUrl);
        setPreviewUrlLoading(false);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setPreviewUrlError(err instanceof Error ? err.message : 'Failed to load preview');
          setPreviewUrlLoading(false);
        }
      });

    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [downloadUrl, mime]);

  return (
    <div className="rounded-md border overflow-hidden bg-background">
      <div className="px-2 py-1 border-b bg-muted/30 flex items-center justify-between">
        <span className="text-xs text-muted-foreground">{title}</span>
        <button
          onClick={() => setExpanded(v => !v)}
          className="text-xs px-2 py-1 rounded border border-border hover:bg-accent transition-colors"
        >
          {expanded
            ? t('smartLog.collapseText', 'Collapse')
            : t('smartLog.expandMore', 'Expand')}
        </button>
      </div>
      <div className={expanded ? PREVIEW_EXPANDED_HEIGHT : PREVIEW_COMPACT_HEIGHT}>
        <div className="h-full overflow-hidden p-2">
          {previewUrlLoading ? (
            <div className="h-full flex items-center justify-center text-xs text-muted-foreground">
              {t('common.loading', 'Loading...')}
            </div>
          ) : previewUrlError ? (
            <div className="h-full flex items-center justify-center text-xs text-destructive">
              {previewUrlError}
            </div>
          ) : (
            <SharedPreviewContent
              document={{
                name: artifact.name || artifact.file_path || 'artifact',
                mime_type: mime,
                file_size: artifact.size || 0,
              }}
              contentUrl={previewUrl}
            />
          )}
        </div>
      </div>
    </div>
  );
}
