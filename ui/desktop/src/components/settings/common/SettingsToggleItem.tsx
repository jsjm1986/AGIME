import React from 'react';
import { cn } from '../../../utils';
import { Switch } from '../../ui/switch';

interface SettingsToggleItemProps {
  title: string;
  description?: React.ReactNode;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
  className?: string;
  children?: React.ReactNode;
}

/**
 * SettingsToggleItem - 开关设置项组件
 *
 * 带开关控件的设置项，支持展开子内容
 *
 * 设计规范:
 * - 标题: 14px medium (text-sm font-medium)
 * - 描述: 12px muted (text-xs text-text-muted)
 * - 开关在右侧
 * - 开启时可展开子内容
 */
export const SettingsToggleItem: React.FC<SettingsToggleItemProps> = ({
  title,
  description,
  checked,
  onCheckedChange,
  disabled = false,
  className,
  children,
}) => {
  return (
    <div className={cn('space-y-0', className)}>
      <div
        className={cn(
          'py-2 px-2 rounded-lg transition-colors duration-200',
          'hover:bg-background-muted',
          disabled && 'opacity-50'
        )}
      >
        <div className="flex items-center justify-between gap-4">
          <div className="flex-1 min-w-0">
            <h4 className="text-sm font-medium text-text-default leading-5">
              {title}
            </h4>
            {description && (
              <p className="text-xs text-text-muted mt-0.5 leading-4 max-w-md">
                {description}
              </p>
            )}
          </div>
          <div className="flex-shrink-0">
            <Switch
              checked={checked}
              onCheckedChange={onCheckedChange}
              disabled={disabled}
              variant="mono"
            />
          </div>
        </div>
      </div>

      {/* Expanded Content when checked */}
      {children && (
        <div
          className={cn(
            'overflow-hidden transition-all duration-300 ease-in-out',
            checked ? 'max-h-[1000px] opacity-100' : 'max-h-0 opacity-0'
          )}
        >
          <div className="mt-3 px-2 space-y-3">{children}</div>
        </div>
      )}
    </div>
  );
};

export default SettingsToggleItem;
