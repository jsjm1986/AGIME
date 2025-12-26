import React from 'react';
import { cn } from '../../../utils';

interface SettingsCardProps {
  icon?: React.ReactNode;
  title: string;
  description?: string;
  children: React.ReactNode;
  className?: string;
  headerClassName?: string;
  contentClassName?: string;
}

/**
 * SettingsCard - Material Design 风格的设置卡片组件
 *
 * 设计规范:
 * - 区块标题: 16px semibold (text-base font-semibold)
 * - 区块描述: 12px muted (text-xs text-text-muted)
 * - 标题-描述间距: 4px (mt-1)
 * - 头部-内容间距: 20px (pt-5)
 * - 内容区间距: 12px (space-y-3)
 * - 卡片内边距: 16px (p-4)
 */
export const SettingsCard: React.FC<SettingsCardProps> = ({
  icon,
  title,
  description,
  children,
  className,
  headerClassName,
  contentClassName,
}) => {
  return (
    <div
      className={cn(
        // 基础样式
        'rounded-xl border border-border-default bg-background-card',
        // 浅色模式阴影
        'shadow-[0_1px_3px_rgba(0,0,0,0.08)]',
        // 深色模式无阴影
        'dark:shadow-none',
        className
      )}
    >
      {/* Header */}
      <div className={cn('p-4 pb-0', headerClassName)}>
        <div className={cn('flex', icon && 'items-start gap-3')}>
          {icon && (
            <div className="flex-shrink-0 w-5 h-5 text-text-muted mt-0.5">
              {icon}
            </div>
          )}
          <div className="flex-1 min-w-0">
            <h3 className="text-base font-semibold text-text-default leading-6">
              {title}
            </h3>
            {description && (
              <p className="text-xs text-text-muted mt-1 leading-4 max-w-2xl">
                {description}
              </p>
            )}
          </div>
        </div>
      </div>

      {/* Content */}
      <div className={cn('p-4 pt-5 space-y-3', contentClassName)}>
        {children}
      </div>
    </div>
  );
};

export default SettingsCard;
