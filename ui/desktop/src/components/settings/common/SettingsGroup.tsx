import React from 'react';
import { cn } from '../../../utils';

interface SettingsGroupProps {
  title?: string;
  description?: string;
  children: React.ReactNode;
  className?: string;
}

/**
 * SettingsGroup - 设置项分组组件
 *
 * 用于在 SettingsCard 内部对设置项进行分组
 *
 * 设计规范:
 * - 分组标题: 12px medium (text-xs font-medium)
 * - 分组描述: 11px muted
 * - 子项间距: 8px (space-y-2)
 */
export const SettingsGroup: React.FC<SettingsGroupProps> = ({
  title,
  description,
  children,
  className,
}) => {
  return (
    <div className={cn('space-y-2', className)}>
      {(title || description) && (
        <div className="px-2">
          {title && (
            <h5 className="text-xs font-medium text-text-default leading-4">
              {title}
            </h5>
          )}
          {description && (
            <p className="text-[11px] text-text-muted mt-0.5 leading-[14px]">
              {description}
            </p>
          )}
        </div>
      )}
      <div className="space-y-1">{children}</div>
    </div>
  );
};

export default SettingsGroup;
