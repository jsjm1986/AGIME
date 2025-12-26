import React, { useState } from 'react';
import { cn } from '../../../utils';
import { ChevronDown } from 'lucide-react';

interface SettingsItemProps {
  title: string;
  description?: React.ReactNode;
  control?: React.ReactNode;
  children?: React.ReactNode;
  onClick?: () => void;
  className?: string;
  expandable?: boolean;
  defaultExpanded?: boolean;
  selected?: boolean;
  disabled?: boolean;
}

/**
 * SettingsItem - Material Design 风格的设置项组件
 *
 * 设计规范:
 * - 子项标题: 14px medium (text-sm font-medium)
 * - 子项描述: 12px muted (text-xs text-text-muted)
 * - 标题-描述间距: 2px (mt-0.5)
 * - 内边距: 8px (py-2 px-2)
 * - 悬停背景: bg-background-muted
 */
export const SettingsItem: React.FC<SettingsItemProps> = ({
  title,
  description,
  control,
  children,
  onClick,
  className,
  expandable = false,
  defaultExpanded = false,
  selected = false,
  disabled = false,
}) => {
  const [isExpanded, setIsExpanded] = useState(defaultExpanded);

  const handleClick = () => {
    if (disabled) return;
    if (expandable) {
      setIsExpanded(!isExpanded);
    }
    onClick?.();
  };

  const isClickable = onClick || expandable;

  return (
    <div className={cn('space-y-0', className)}>
      <div
        className={cn(
          'py-2 px-2 rounded-lg transition-colors duration-200',
          // 悬停效果
          !disabled && 'hover:bg-background-muted',
          // 选中状态
          selected && 'bg-background-muted',
          // 可点击状态
          isClickable && !disabled && 'cursor-pointer',
          // 禁用状态
          disabled && 'opacity-50 cursor-not-allowed'
        )}
        onClick={handleClick}
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
          <div className="flex-shrink-0 flex items-center gap-2">
            {control}
            {expandable && (
              <ChevronDown
                className={cn(
                  'w-4 h-4 text-text-muted transition-transform duration-200',
                  isExpanded && 'rotate-180'
                )}
              />
            )}
          </div>
        </div>
      </div>

      {/* Expanded Content */}
      {children && (
        <div
          className={cn(
            'overflow-hidden transition-all duration-300 ease-in-out',
            isExpanded || !expandable
              ? 'max-h-[1000px] opacity-100'
              : 'max-h-0 opacity-0'
          )}
        >
          <div className="mt-3 px-2 space-y-3">{children}</div>
        </div>
      )}
    </div>
  );
};

export default SettingsItem;
