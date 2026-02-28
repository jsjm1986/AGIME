import { useTranslation } from 'react-i18next';
import { Button } from '../ui/button';
import { documentApi } from '../../api/documents';
import type { DocumentSummary } from '../../api/documents';
import { TextPreview } from './previews/TextPreview';
import { MarkdownPreview } from './previews/MarkdownPreview';
import { ImagePreview } from './previews/ImagePreview';
import { MediaPreview } from './previews/MediaPreview';
import { FallbackPreview } from './previews/FallbackPreview';
import { WordPreview } from './previews/WordPreview';
import { ExcelPreview } from './previews/ExcelPreview';
import { PptPreview } from './previews/PptPreview';
import { HtmlPreview } from './previews/HtmlPreview';
import { CsvPreview } from './previews/CsvPreview';
import { SvgPreview } from './previews/SvgPreview';
import { FontPreview } from './previews/FontPreview';
import { JsonPreview } from './previews/JsonPreview';

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

export function SharedPreviewContent({ document: doc, contentUrl, onDownload }: PreviewContentProps) {
  const mime = doc.mime_type || '';

  // Markdown
  if (mime === 'text/markdown') {
    return <MarkdownPreview contentUrl={contentUrl} />;
  }

  // HTML
  if (mime === 'text/html') {
    return <HtmlPreview contentUrl={contentUrl} />;
  }

  // CSV
  if (mime === 'text/csv' || isCsvName(doc.name)) {
    return <CsvPreview contentUrl={contentUrl} />;
  }

  // JSON
  if (mime === 'application/json' || isJsonName(doc.name)) {
    return <JsonPreview contentUrl={contentUrl} />;
  }

  // Text-based files
  if (isTextType(mime)) {
    return <TextPreview contentUrl={contentUrl} mimeType={mime} />;
  }

  // SVG (before generic image)
  if (mime === 'image/svg+xml') {
    return <SvgPreview contentUrl={contentUrl} />;
  }

  // Images
  if (mime.startsWith('image/')) {
    return <ImagePreview contentUrl={contentUrl} fileName={doc.name} />;
  }

  // Fonts
  if (mime.startsWith('font/') || isFontName(doc.name)) {
    return <FontPreview contentUrl={contentUrl} />;
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
    return <MediaPreview contentUrl={contentUrl} mimeType={mime} />;
  }

  // Word documents
  if (
    mime === 'application/vnd.openxmlformats-officedocument.wordprocessingml.document' ||
    mime === 'application/msword'
  ) {
    return <WordPreview contentUrl={contentUrl} />;
  }

  // Excel spreadsheets
  if (
    mime === 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' ||
    mime === 'application/vnd.ms-excel'
  ) {
    return <ExcelPreview contentUrl={contentUrl} />;
  }

  // PowerPoint presentations
  if (
    mime === 'application/vnd.openxmlformats-officedocument.presentationml.presentation' ||
    mime === 'application/vnd.ms-powerpoint'
  ) {
    return <PptPreview contentUrl={contentUrl} />;
  }

  // Fallback
  return <FallbackPreview document={doc} onDownload={onDownload} />;
}
