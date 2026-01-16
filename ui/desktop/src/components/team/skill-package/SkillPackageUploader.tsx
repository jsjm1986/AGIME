import { useState, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Upload, Package, AlertCircle, CheckCircle2, Loader2, X } from 'lucide-react';
import { Button } from '../../ui/button';
import { validateSkillPackage } from '../api';
import { formatPackageSize } from '../types';

interface ParsedPackageInfo {
  name: string;
  description: string;
  fileCount: number;
  totalSize: number;
}

interface ValidationResult {
  valid: boolean;
  errors: string[];
  warnings: string[];
  parsed?: ParsedPackageInfo;
}

interface SkillPackageUploaderProps {
  onFileSelected: (file: File, validation: ValidationResult) => void;
  onClear?: () => void;
  disabled?: boolean;
  selectedFile?: File | null;
  validationResult?: ValidationResult | null;
}

export function SkillPackageUploader({
  onFileSelected,
  onClear,
  disabled = false,
  selectedFile,
  validationResult,
}: SkillPackageUploaderProps) {
  const { t } = useTranslation('team');
  const [isDragging, setIsDragging] = useState(false);
  const [isValidating, setIsValidating] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleFile = useCallback(async (file: File) => {
    setLocalError(null);

    // Check file type
    if (!file.name.endsWith('.zip')) {
      setLocalError(t('skillPackage.invalidFormat', '请上传 ZIP 格式的文件'));
      return;
    }

    // Check file size (10 MB max)
    if (file.size > 10 * 1024 * 1024) {
      setLocalError(t('skillPackage.fileTooLarge', '文件大小不能超过 10 MB'));
      return;
    }

    setIsValidating(true);
    try {
      const result = await validateSkillPackage(file);
      onFileSelected(file, result);
    } catch (err) {
      setLocalError(err instanceof Error ? err.message : t('skillPackage.validationFailed', '验证失败'));
    } finally {
      setIsValidating(false);
    }
  }, [onFileSelected, t]);

  const handleDrop = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);

    if (disabled || isValidating) return;

    const file = e.dataTransfer.files[0];
    if (file) {
      handleFile(file);
    }
  }, [disabled, isValidating, handleFile]);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    if (!disabled && !isValidating) {
      setIsDragging(true);
    }
  }, [disabled, isValidating]);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    setIsDragging(false);
  }, []);

  const handleClick = () => {
    if (!disabled && !isValidating) {
      fileInputRef.current?.click();
    }
  };

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) {
      handleFile(file);
    }
    // Reset input so same file can be selected again
    e.target.value = '';
  };

  const handleClear = () => {
    setLocalError(null);
    onClear?.();
  };

  // Show selected file state
  if (selectedFile && validationResult) {
    return (
      <div className="border rounded-lg p-4 bg-background-muted">
        <div className="flex items-start justify-between">
          <div className="flex items-start gap-3">
            <div className={`p-2 rounded-lg ${validationResult.valid ? 'bg-green-500/10' : 'bg-red-500/10'}`}>
              {validationResult.valid ? (
                <CheckCircle2 className="h-5 w-5 text-green-500" />
              ) : (
                <AlertCircle className="h-5 w-5 text-red-500" />
              )}
            </div>
            <div className="flex-1 min-w-0">
              <p className="font-medium text-text-default truncate">
                {validationResult.parsed?.name || selectedFile.name}
              </p>
              <p className="text-sm text-text-muted mt-0.5">
                {formatPackageSize(selectedFile.size)}
                {validationResult.parsed && (
                  <span className="ml-2">
                    {t('skillPackage.fileCount', '{{count}} 个文件', { count: validationResult.parsed.fileCount })}
                  </span>
                )}
              </p>
              {validationResult.parsed?.description && (
                <p className="text-sm text-text-muted mt-1 line-clamp-2">
                  {validationResult.parsed.description}
                </p>
              )}
            </div>
          </div>
          <Button
            variant="ghost"
            size="sm"
            onClick={handleClear}
            disabled={disabled}
            className="shrink-0"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* Errors */}
        {validationResult.errors.length > 0 && (
          <div className="mt-3 p-2 bg-red-500/10 rounded text-sm">
            <p className="font-medium text-red-500 mb-1">
              {t('skillPackage.errors', '错误')}:
            </p>
            <ul className="list-disc list-inside text-red-500 space-y-0.5">
              {validationResult.errors.map((err, i) => (
                <li key={i}>{err}</li>
              ))}
            </ul>
          </div>
        )}

        {/* Warnings */}
        {validationResult.warnings.length > 0 && (
          <div className="mt-3 p-2 bg-yellow-500/10 rounded text-sm">
            <p className="font-medium text-yellow-600 mb-1">
              {t('skillPackage.warnings', '警告')}:
            </p>
            <ul className="list-disc list-inside text-yellow-600 space-y-0.5">
              {validationResult.warnings.map((warn, i) => (
                <li key={i}>{warn}</li>
              ))}
            </ul>
          </div>
        )}
      </div>
    );
  }

  // Show upload dropzone
  return (
    <div
      className={`
        relative border-2 border-dashed rounded-lg p-6 text-center cursor-pointer
        transition-colors duration-200
        ${isDragging ? 'border-teal-500 bg-teal-500/5' : 'border-border-default'}
        ${disabled || isValidating ? 'opacity-50 cursor-not-allowed' : 'hover:border-teal-500/50'}
      `}
      onDrop={handleDrop}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onClick={handleClick}
    >
      <input
        ref={fileInputRef}
        type="file"
        accept=".zip"
        onChange={handleInputChange}
        className="hidden"
        disabled={disabled || isValidating}
      />

      {isValidating ? (
        <div className="flex flex-col items-center gap-2 py-4">
          <Loader2 className="h-8 w-8 text-teal-500 animate-spin" />
          <p className="text-sm text-text-muted">
            {t('skillPackage.validating', '正在验证包...')}
          </p>
        </div>
      ) : (
        <div className="flex flex-col items-center gap-2 py-4">
          <div className="p-3 bg-background-muted rounded-full">
            {isDragging ? (
              <Package className="h-8 w-8 text-teal-500" />
            ) : (
              <Upload className="h-8 w-8 text-text-muted" />
            )}
          </div>
          <div>
            <p className="text-sm font-medium text-text-default">
              {t('skillPackage.dropZone', '拖拽 ZIP 文件到此处')}
            </p>
            <p className="text-xs text-text-muted mt-1">
              {t('skillPackage.orClick', '或点击选择文件')}
            </p>
          </div>
          <p className="text-xs text-text-muted">
            {t('skillPackage.maxSize', '最大 10 MB')}
          </p>
        </div>
      )}

      {localError && (
        <div className="mt-3 p-2 bg-red-500/10 rounded">
          <p className="text-sm text-red-500 flex items-center gap-1.5">
            <AlertCircle className="h-4 w-4 shrink-0" />
            {localError}
          </p>
        </div>
      )}
    </div>
  );
}

export default SkillPackageUploader;
