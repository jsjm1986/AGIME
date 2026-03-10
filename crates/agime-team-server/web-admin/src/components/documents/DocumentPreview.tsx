import { lazy, Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { documentApi } from '../../api/documents';
import type { DocumentSummary } from '../../api/documents';
import { FallbackPreview } from './previews/FallbackPreview';

const TextPreview = lazy(() =>
  import('./previews/TextPreview').then((module) => ({ default: module.TextPreview })),
);
const MarkdownPreview = lazy(() =>
  import('./previews/MarkdownPreview').then((module) => ({ default: module.MarkdownPreview })),
);
const ImagePreview = lazy(() =>
  import('./previews/ImagePreview').then((module) => ({ default: module.ImagePreview })),
);
const MediaPreview = lazy(() =>
  import('./previews/MediaPreview').then((module) => ({ default: module.MediaPreview })),
);
const WordPreview = lazy(() =>
  import('./previews/WordPreview').then((module) => ({ default: module.WordPreview })),
);
const ExcelPreview = lazy(() =>
  import('./previews/ExcelPreview').then((module) => ({ default: module.ExcelPreview })),
);
const PptPreview = lazy(() =>
  import('./previews/PptPreview').then((module) => ({ default: module.PptPreview })),
);
const HtmlPreview = lazy(() =>
  import('./previews/HtmlPreview').then((module) => ({ default: module.HtmlPreview })),
);
const CsvPreview = lazy(() =>
  import('./previews/CsvPreview').then((module) => ({ default: module.CsvPreview })),
);
const SvgPreview = lazy(() =>
  import('./previews/SvgPreview').then((module) => ({ default: module.SvgPreview })),
);
const FontPreview = lazy(() =>
  import('./previews/FontPreview').then((module) => ({ default: module.FontPreview })),
);
const JsonPreview = lazy(() =>
  import('./previews/JsonPreview').then((module) => ({ default: module.JsonPreview })),
);

// Editable MIME types
export const EDITABLE_MIME_TYPES = [
  'text/plain', 'text/markdown', 'text/csv', 'text/html', 'text/css',
  'text/javascript', 'application/json', 'application/xml',
  'application/x-yaml', 'text/x-python', 'text/x-rust', 'text/x-go',
  'text/x-java', 'text/x-typescript',
];

// Text-based MIME types for preview
const TEXT_MIME_TYPES = [
  ...EDITABLE_MIME_TYPES,
  'application/javascript', 'application/typescript',
];

function isTextType(mime: string): boolean {
  return mime.startsWith('text/') || TEXT_MIME_TYPES.includes(mime);
}

function isEditable(mime: string): boolean {
  return EDITABLE_MIME_TYPES.includes(mime);
}

interface DocumentPreviewProps {
  teamId: string;
  document: DocumentSummary;
  onClose: () => void;
  onEdit?: () => void;
  onVersions?: () => void;
}

export function DocumentPreview({
  teamId,
  document: doc,
  onClose,
  onEdit,
  onVersions,
}: DocumentPreviewProps) {
  const { t } = useTranslation();
  const contentUrl = documentApi.getContentUrl(teamId, doc.id);

  const handleDownload = () => {
    window.open(documentApi.getDownloadUrl(teamId, doc.id), '_blank');
  };

  return (
    <div className="flex flex-col h-full border-l">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-4 py-2 border-b bg-muted/30">
        <span className="font-medium truncate text-sm flex-1 mr-2">
          {doc.display_name || doc.name}
        </span>
        <div className="flex items-center gap-1 flex-shrink-0">
          {onVersions && isTextType(doc.mime_type) && (
            <Button size="sm" variant="ghost" onClick={onVersions}>
              {t('documents.versions')}
            </Button>
          )}
          {onEdit && isEditable(doc.mime_type) && (
            <Button size="sm" variant="ghost" onClick={onEdit}>
              {t('documents.edit')}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={handleDownload}>
            {t('documents.download')}
          </Button>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t('documents.closePreview')}
          </Button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-hidden">
        <SharedPreviewContent
          document={doc}
          contentUrl={contentUrl}
          onDownload={handleDownload}
        />
      </div>
    </div>
  );
}

interface PreviewContentProps {
  document: {
    name: string;
    mime_type: string;
    file_size: number;
  };
  contentUrl: string;
  onDownload?: () => void;
}

function isCsvName(name: string): boolean {
  return /\.csv$/i.test(name);
}

function isJsonName(name: string): boolean {
  return /\.json$/i.test(name);
}

function isFontName(name: string): boolean {
  return /\.(ttf|otf|woff2?)$/i.test(name);
}

function PreviewLoadingFallback() {
  return (
    <div className="p-4 text-sm text-muted-foreground">
      Loading preview...
    </div>
  );
}

export function SharedPreviewContent({ document: doc, contentUrl, onDownload }: PreviewContentProps) {
  const mime = doc.mime_type || '';

  // Markdown
  if (mime === 'text/markdown') {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <MarkdownPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // HTML
  if (mime === 'text/html') {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <HtmlPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // CSV
  if (mime === 'text/csv' || isCsvName(doc.name)) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <CsvPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // JSON
  if (mime === 'application/json' || isJsonName(doc.name)) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <JsonPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // Text-based files
  if (isTextType(mime)) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <TextPreview contentUrl={contentUrl} mimeType={mime} />
      </Suspense>
    );
  }

  // SVG (before generic image)
  if (mime === 'image/svg+xml') {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <SvgPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // Images
  if (mime.startsWith('image/')) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <ImagePreview contentUrl={contentUrl} fileName={doc.name} />
      </Suspense>
    );
  }

  // Fonts
  if (mime.startsWith('font/') || isFontName(doc.name)) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <FontPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // PDF
  if (mime === 'application/pdf') {
    return (
      <iframe
        src={contentUrl}
        className="w-full h-full border-0"
        title={doc.name}
      />
    );
  }

  // Audio/Video
  if (mime.startsWith('audio/') || mime.startsWith('video/')) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <MediaPreview contentUrl={contentUrl} mimeType={mime} />
      </Suspense>
    );
  }

  // Word documents
  if (
    mime === 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' ||
    mime === 'application/msword'
  ) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <WordPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // Excel spreadsheets
  if (
    mime === 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' ||
    mime === 'application/vnd.ms-excel'
  ) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <ExcelPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // PowerPoint presentations
  if (
    mime === 'application/vnd.openxmlformats-officedocument.presentationml.presentation' ||
    mime === 'application/vnd.ms-powerpoint'
  ) {
    return (
      <Suspense fallback={<PreviewLoadingFallback />}>
        <PptPreview contentUrl={contentUrl} />
      </Suspense>
    );
  }

  // Fallback
  return <FallbackPreview document={doc} onDownload={onDownload} />;
}
