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

// Constants for file handling
const MAX_IMAGE_SIZE_MB = 5;
const MAX_FILES_PER_DROP = 10;
const SUPPORTED_IMAGE_TYPES = ['image/png', 'image/jpeg', 'image/jpg', 'image/gif', 'image/webp', 'image/bmp'];

export const useFileDrop = () => {
  const [droppedFiles, setDroppedFiles] = useState<DroppedFile[]>([]);
  const activeReadersRef = useRef<Set<FileReader>>(new Set());
  // 用于跟踪组件是否已卸载，防止在卸载后更新状态
  const isMountedRef = useRef<boolean>(true);

  // Cleanup effect to prevent memory leaks
  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
      // Abort any active FileReaders on unmount
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
    e.stopPropagation();

    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;

    // 限制单次拖放的文件数量
    const filesToProcess = Array.from(files).slice(0, MAX_FILES_PER_DROP);
    if (files.length > MAX_FILES_PER_DROP) {
      console.warn(`[useFileDrop] Too many files dropped (${files.length}), only processing first ${MAX_FILES_PER_DROP}`);
    }

    // 阶段 1：收集所有文件对象和待处理的图片
    const droppedFileObjects: DroppedFile[] = [];
    const imagesToRead: { file: File; fileId: string }[] = [];

    for (let i = 0; i < filesToProcess.length; i++) {
      const file = filesToProcess[i];
      const isImage = file.type.startsWith('image/');
      const fileId = `dropped-${Date.now()}-${i}-${Math.random().toString(36).substring(2, 7)}`;

      // 检查图片大小
      if (isImage && file.size > MAX_IMAGE_SIZE_MB * 1024 * 1024) {
        droppedFileObjects.push({
          id: fileId,
          path: '',
          name: file.name,
          type: file.type,
          isImage: true,
          isLoading: false,
          error: `Image too large (${Math.round(file.size / (1024 * 1024))}MB, max ${MAX_IMAGE_SIZE_MB}MB)`,
        });
        continue;
      }

      // 检查图片格式是否支持
      if (isImage && !SUPPORTED_IMAGE_TYPES.includes(file.type.toLowerCase())) {
        droppedFileObjects.push({
          id: fileId,
          path: '',
          name: file.name,
          type: file.type,
          isImage: true,
          isLoading: false,
          error: `Unsupported image format: ${file.type}`,
        });
        continue;
      }

      let filePath: string;
      try {
        // Web 模式下图片使用文件名，因为 getPathForFile 只返回文件名
        // Electron 模式下使用完整路径
        if (isWeb) {
          filePath = isImage ? file.name : file.name;
        } else {
          filePath = window.electron?.getPathForFile?.(file) || file.name;
        }
      } catch (error) {
        console.error('[useFileDrop] Error getting file path:', file.name, error);
        droppedFileObjects.push({
          id: fileId,
          path: '',
          name: file.name,
          type: file.type,
          isImage: false,
          isLoading: false,
          error: `Failed to get file path: ${error instanceof Error ? error.message : 'Unknown error'}`,
        });
        continue;
      }

      const droppedFile: DroppedFile = {
        id: fileId,
        path: filePath,
        name: file.name,
        type: file.type,
        isImage,
        isLoading: isImage, // Only images need loading state for preview generation
      };

      droppedFileObjects.push(droppedFile);

      // 记录需要读取的图片
      if (isImage) {
        imagesToRead.push({ file, fileId });
      }
    }

    // 阶段 2：先将所有文件添加到状态，确保 FileReader.onload 能找到对应的文件
    // 这是修复竞态条件的关键！
    if (droppedFileObjects.length > 0) {
      setDroppedFiles((prev) => [...prev, ...droppedFileObjects]);
    }

    // 阶段 3：开始异步读取图片（此时文件已经在状态中）
    for (const { file, fileId } of imagesToRead) {
      // 检查组件是否仍然挂载
      if (!isMountedRef.current) {
        console.log('[useFileDrop] Component unmounted, skipping file read');
        break;
      }

      const reader = new FileReader();
      activeReadersRef.current.add(reader);

      reader.onload = (event) => {
        // 防止在组件卸载后更新状态
        if (!isMountedRef.current) {
          activeReadersRef.current.delete(reader);
          return;
        }

        const dataUrl = event.target?.result;
        if (typeof dataUrl === 'string' && dataUrl.startsWith('data:image/')) {
          setDroppedFiles((prev) =>
            prev.map((f) => (f.id === fileId ? { ...f, dataUrl, isLoading: false } : f))
          );
        } else {
          // dataUrl 格式无效
          console.error('[useFileDrop] Invalid dataUrl format for:', file.name);
          setDroppedFiles((prev) =>
            prev.map((f) =>
              f.id === fileId
                ? { ...f, error: 'Failed to read image: invalid format', isLoading: false }
                : f
            )
          );
        }
        activeReadersRef.current.delete(reader);
      };

      reader.onerror = (event) => {
        if (!isMountedRef.current) {
          activeReadersRef.current.delete(reader);
          return;
        }

        const errorMessage = event.target?.error?.message || 'Unknown read error';
        console.error('[useFileDrop] Failed to read file:', file.name, errorMessage);
        setDroppedFiles((prev) =>
          prev.map((f) =>
            f.id === fileId
              ? { ...f, error: `Failed to load image: ${errorMessage}`, isLoading: false }
              : f
          )
        );
        activeReadersRef.current.delete(reader);
      };

      reader.onabort = () => {
        if (!isMountedRef.current) {
          activeReadersRef.current.delete(reader);
          return;
        }

        console.log('[useFileDrop] File read aborted:', file.name);
        setDroppedFiles((prev) =>
          prev.map((f) =>
            f.id === fileId
              ? { ...f, error: 'Image loading was cancelled', isLoading: false }
              : f
          )
        );
        activeReadersRef.current.delete(reader);
      };

      try {
        reader.readAsDataURL(file);
      } catch (error) {
        console.error('[useFileDrop] Error starting file read:', file.name, error);
        activeReadersRef.current.delete(reader);
        if (isMountedRef.current) {
          setDroppedFiles((prev) =>
            prev.map((f) =>
              f.id === fileId
                ? { ...f, error: `Failed to start reading: ${error instanceof Error ? error.message : 'Unknown error'}`, isLoading: false }
                : f
            )
          );
        }
      }
    }
  }, []);

  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
  }, []);

  // 清除所有文件的方法
  const clearFiles = useCallback(() => {
    // 中止所有正在进行的读取
    activeReadersRef.current.forEach((reader) => {
      try {
        reader.abort();
      } catch {
        // Ignore errors
      }
    });
    activeReadersRef.current.clear();
    setDroppedFiles([]);
  }, []);

  // 移除单个文件的方法
  const removeFile = useCallback((fileId: string) => {
    setDroppedFiles((prev) => prev.filter((f) => f.id !== fileId));
  }, []);

  return {
    droppedFiles,
    setDroppedFiles,
    handleDrop,
    handleDragOver,
    clearFiles,
    removeFile,
  };
};
