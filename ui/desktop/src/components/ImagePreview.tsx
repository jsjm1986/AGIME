import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { isWeb } from '../platform';

interface ImagePreviewProps {
  src: string;
  alt?: string;
  className?: string;
}

export default function ImagePreview({
  src,
  alt,
  className = '',
}: ImagePreviewProps) {
  const { t } = useTranslation('errors');
  const effectiveAlt = alt ?? t('pastedImage');
  const [isExpanded, setIsExpanded] = useState(false);
  const [error, setError] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [imageData, setImageData] = useState<string | null>(null);

  // Determine the type of image source
  const isDataUrl = src.startsWith('data:image/');
  const isBlobUrl = src.startsWith('blob:');
  const isFilePath = src.includes('agime-pasted-images') || src.includes('goose-pasted-images');

  useEffect(() => {
    const loadImage = async () => {
      try {
        // For data URLs and blob URLs, use directly
        if (isDataUrl || isBlobUrl) {
          setImageData(src);
          setIsLoading(false);
          return;
        }

        // For file paths, use IPC to load (Electron only)
        if (isFilePath) {
          if (isWeb) {
            // In Web mode, file paths won't work - show error
            console.warn('[ImagePreview] File path not supported in Web mode:', src);
            setError(true);
            setIsLoading(false);
            return;
          }

          const data = await window.electron.getTempImage(src);
          if (data) {
            setImageData(data);
            setIsLoading(false);
          } else {
            setError(true);
            setIsLoading(false);
          }
          return;
        }

        // Unknown format - try to use as-is (for backwards compatibility)
        setImageData(src);
        setIsLoading(false);
      } catch (err) {
        console.error('Error loading image:', err);
        setError(true);
        setIsLoading(false);
      }
    };

    loadImage();
  }, [src, isDataUrl, isBlobUrl, isFilePath]);

  const handleError = () => {
    setError(true);
    setIsLoading(false);
  };

  const toggleExpand = () => {
    if (!error) {
      setIsExpanded(!isExpanded);
    }
  };

  // Validate image source - allow data URLs, blob URLs, and known file paths
  if (!isDataUrl && !isBlobUrl && !isFilePath) {
    return <div className="text-red-500 text-xs italic mt-1 mb-1">{t('invalidImagePath', { path: src.substring(0, 50) + '...' })}</div>;
  }

  if (error) {
    return <div className="text-red-500 text-xs italic mt-1 mb-1">{t('unableToLoadImage', { path: src.substring(0, 50) + '...' })}</div>;
  }

  return (
    <div className={`image-preview mt-2 mb-2 ${className}`}>
      {isLoading && (
        <div className="animate-pulse bg-gray-200 rounded w-40 h-40 flex items-center justify-center">
          <span className="text-gray-500 text-xs">{t('loading')}</span>
        </div>
      )}
      {imageData && (
        <img
          src={imageData}
          alt={effectiveAlt}
          onError={handleError}
          onClick={toggleExpand}
          className={`rounded border border-borderSubtle cursor-pointer hover:border-borderStandard transition-all ${
            isExpanded ? 'max-w-full max-h-96' : 'max-h-40 max-w-40'
          } ${isLoading ? 'hidden' : ''}`}
          style={{ objectFit: 'contain' }}
        />
      )}
      {isExpanded && !error && !isLoading && imageData && (
        <div className="text-xs text-textSubtle mt-1">{t('clickToCollapse')}</div>
      )}
      {!isExpanded && !error && !isLoading && imageData && (
        <div className="text-xs text-textSubtle mt-1">{t('clickToExpand')}</div>
      )}
    </div>
  );
}
