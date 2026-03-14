import { ReactNode } from 'react';
import { Card, CardContent } from '../ui/card';

interface StatsCardProps {
  title: string;
  value: string | number;
  icon: ReactNode;
  description?: string;
  trend?: {
    value: number;
    isPositive: boolean;
  };
}

export function StatsCard({ title, value, icon, description, trend }: StatsCardProps) {
  return (
    <Card className="border-[hsl(var(--ui-line-soft))/0.82] bg-[hsl(var(--ui-surface-panel))/0.88] shadow-[0_14px_34px_hsl(var(--ui-shadow)/0.08)]">
      <CardContent className="p-6">
        <div className="flex items-center justify-between">
          <div className="space-y-1">
            <p className="text-[11px] font-semibold uppercase tracking-[0.08em] text-[hsl(var(--muted-foreground))/0.84]">{title}</p>
            <p className="text-[28px] font-semibold tracking-[-0.04em] text-[hsl(var(--foreground))]">{value}</p>
            {description && (
              <p className="text-xs leading-5 text-[hsl(var(--muted-foreground))/0.9]">{description}</p>
            )}
            {trend && (
              <p className={`text-xs font-medium ${trend.isPositive ? 'text-[hsl(var(--status-success-text))]' : 'text-[hsl(var(--status-error-text))]'}`}>
                {trend.isPositive ? '+' : ''}{trend.value}%
              </p>
            )}
          </div>
          <div className="flex h-12 w-12 items-center justify-center rounded-[16px] border border-[hsl(var(--ui-line-soft))/0.68] bg-[hsl(var(--ui-surface-panel-strong))/0.82] text-[hsl(var(--primary))]">
            {icon}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
