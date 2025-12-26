import React from 'react';
import { cn } from '../../../utils';

interface SettingsRadioItemProps {
  title: string;
  description?: string;
  selected: boolean;
  onSelect: () => void;
  disabled?: boolean;
  className?: string;
}

/**
 * SettingsRadioItem - 单选设置项组件
 *
 * 用于模式选择等场景
 *
 * 设计规范:
 * - 标题: 14px medium (text-sm font-medium)
 * - 描述: 12px muted (text-xs text-text-muted)
 * - 选中时左侧显示强调色边框
 */
export const SettingsRadioItem: React.FC<SettingsRadioItemProps> = ({
  title,
  description,
  selected,
  onSelect,
  disabled = false,
  className,
}) => {
  return (
    <div
      className={cn(
        'py-2 px-3 rounded-lg transition-all duration-200',
        'border-l-2',
        // 选中状态
        selected
          ? 'bg-background-muted border-l-block-teal'
          : 'border-l-transparent hover:bg-background-muted',
        // 可点击
        !disabled && 'cursor-pointer',
        // 禁用状态
        disabled && 'opacity-50 cursor-not-allowed',
        className
      )}
      onClick={() => !disabled && onSelect()}
    >
      <h4 className={cn(
        'text-sm font-medium leading-5',
        selected ? 'text-text-default' : 'text-text-default'
      )}>
        {title}
      </h4>
      {description && (
        <p className="text-xs text-text-muted mt-0.5 leading-4 max-w-md">
          {description}
        </p>
      )}
    </div>
  );
};

export default SettingsRadioItem;
