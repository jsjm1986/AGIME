import { useCallback, useState, useRef, useEffect } from 'react';
import { isWeb } from '../platform';

export interface DroppedFile {
  id: string;
  path: string;           // File path (Electron) or filename (Web for images)
  name: string;           // Original filename
  type: string;           // MIME type
  isImage: boolean;       // Whether this is an image file
  dataUrl?: string;       // Base64 data URL for images (used for both preview and sending)
  isLoading?: boolean;    // Whether the file is being processed
  error?: string;         // Error message if processing failed
}

export const useFileDrop = () => {
  const [droppedFiles, setDroppedFiles] = useState<DroppedFile[]>([]);
  const activeReadersRef = useRef<Set<FileReader>>(new Set());

  // Cleanup effect to prevent memory leaks
  useEffect(() => {
    return () => {
      // Abort any active FileReaders on unmount
      // eslint-disable-next-line react-hooks/exhaustive-deps
      const readers = activeReadersRef.current;
      readers.forEach((reader) => {
        try {
          reader.abort();
        } catch {
          // Reader might already be done, ignore errors
        }
      });
      readers.clear();
    };
  }, []);

  const handleDrop = useCallback(async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    const files = e.dataTransfer.files;
    if (files.length > 0) {
      const droppedFileObjects: DroppedFile[] = [];

      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        const isImage = file.type.startsWith('image/');

        let droppedFile: DroppedFile;

        try {
          // Web 模式下图片使用文件名，因为 getPathForFile 只返回文件名
          // Electron 模式下使用完整路径
          const path = isWeb && isImage ? file.name : window.electron.getPathForFile(file);

          droppedFile = {
            id: `dropped-${Date.now()}-${i}`,
            path,
            name: file.name,
            type: file.type,
            isImage,
            isLoading: isImage, // Only images need loading state for preview generation
          };
        } catch (error) {
          console.error('Error processing file:', file.name, error);
          // Create an error file object
          droppedFile = {
            id: `dropped-error-${Date.now()}-${i}`,
            path: '',
            name: file.name,
            type: file.type,
            isImage: false,
            isLoading: false,
            error: `Failed to get file path: ${error instanceof Error ? error.message : 'Unknown error'}`,
          };
        }

        droppedFileObjects.push(droppedFile);

        // For images, generate a preview (only if successfully processed)
        if (droppedFile.isImage && !droppedFile.error) {
          const reader = new FileReader();
          activeReadersRef.current.add(reader);

          reader.onload = (event) => {
            const dataUrl = event.target?.result as string;
            setDroppedFiles((prev) =>
              prev.map((f) => (f.id === droppedFile.id ? { ...f, dataUrl, isLoading: false } : f))
            );
            activeReadersRef.current.delete(reader);
          };

          reader.onerror = () => {
            console.error('Failed to generate preview for:', file.name);
            setDroppedFiles((prev) =>
              prev.map((f) =>
                f.id === droppedFile.id
                  ? { ...f, error: 'Failed to load image preview', isLoading: false }
                  : f
              )
            );
            activeReadersRef.current.delete(reader);
          };

          reader.onabort = () => {
            activeReadersRef.current.delete(reader);
          };

          reader.readAsDataURL(file);
        }
      }

      setDroppedFiles((prev) => [...prev, ...droppedFileObjects]);
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
  }, []);

  return {
    droppedFiles,
    setDroppedFiles,
    handleDrop,
    handleDragOver,
  };
};
