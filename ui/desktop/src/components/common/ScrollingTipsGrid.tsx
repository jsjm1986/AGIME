import { useState, useMemo } from 'react';
import { cn } from '../../utils';
import { tips, Tip } from './ScrollingTips';

// Category colors matching the design system
const categoryColors: Record<string, { bg: string; border: string; text: string }> = {
  '清理': { bg: 'bg-emerald-100 dark:bg-emerald-500/15', border: 'border-emerald-200 dark:border-emerald-500/30', text: 'text-emerald-700 dark:text-emerald-400' },
  '系统': { bg: 'bg-blue-100 dark:bg-blue-500/15', border: 'border-blue-200 dark:border-blue-500/30', text: 'text-blue-700 dark:text-blue-400' },
  '整理': { bg: 'bg-cyan-100 dark:bg-cyan-500/15', border: 'border-cyan-200 dark:border-cyan-500/30', text: 'text-cyan-700 dark:text-cyan-400' },
  '记忆': { bg: 'bg-purple-100 dark:bg-purple-500/15', border: 'border-purple-200 dark:border-purple-500/30', text: 'text-purple-700 dark:text-purple-400' },
  '文档': { bg: 'bg-amber-100 dark:bg-amber-500/15', border: 'border-amber-200 dark:border-amber-500/30', text: 'text-amber-700 dark:text-amber-400' },
  '数据': { bg: 'bg-green-100 dark:bg-green-500/15', border: 'border-green-200 dark:border-green-500/30', text: 'text-green-700 dark:text-green-400' },
  '截图': { bg: 'bg-pink-100 dark:bg-pink-500/15', border: 'border-pink-200 dark:border-pink-500/30', text: 'text-pink-700 dark:text-pink-400' },
  '办公': { bg: 'bg-orange-100 dark:bg-orange-500/15', border: 'border-orange-200 dark:border-orange-500/30', text: 'text-orange-700 dark:text-orange-400' },
  '生活': { bg: 'bg-rose-100 dark:bg-rose-500/15', border: 'border-rose-200 dark:border-rose-500/30', text: 'text-rose-700 dark:text-rose-400' },
  '社交': { bg: 'bg-red-100 dark:bg-red-500/15', border: 'border-red-200 dark:border-red-500/30', text: 'text-red-700 dark:text-red-400' },
  '学习': { bg: 'bg-indigo-100 dark:bg-indigo-500/15', border: 'border-indigo-200 dark:border-indigo-500/30', text: 'text-indigo-700 dark:text-indigo-400' },
  '自动化': { bg: 'bg-yellow-100 dark:bg-yellow-500/15', border: 'border-yellow-200 dark:border-yellow-500/30', text: 'text-yellow-700 dark:text-yellow-400' },
  '创意': { bg: 'bg-fuchsia-100 dark:bg-fuchsia-500/15', border: 'border-fuchsia-200 dark:border-fuchsia-500/30', text: 'text-fuchsia-700 dark:text-fuchsia-400' },
  '搜索': { bg: 'bg-teal-100 dark:bg-teal-500/15', border: 'border-teal-200 dark:border-teal-500/30', text: 'text-teal-700 dark:text-teal-400' },
  '探索': { bg: 'bg-violet-100 dark:bg-violet-500/15', border: 'border-violet-200 dark:border-violet-500/30', text: 'text-violet-700 dark:text-violet-400' },
};

const defaultColor = { bg: 'bg-black/5 dark:bg-white/5', border: 'border-black/10 dark:border-white/10', text: 'text-text-muted dark:text-white/70' };

// Shuffle array helper
const shuffleArray = <T,>(array: T[]): T[] => {
  const shuffled = [...array];
  for (let i = shuffled.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
  }
  return shuffled;
};

// Split tips into rows
const splitIntoRows = (tips: Tip[], rowCount: number): Tip[][] => {
  const shuffled = shuffleArray(tips);
  const rows: Tip[][] = Array.from({ length: rowCount }, () => []);

  shuffled.forEach((tip, index) => {
    rows[index % rowCount].push(tip);
  });

  return rows;
};

interface TipCardProps {
  tip: Tip;
  onClick: () => void;
}

function TipCard({ tip, onClick }: TipCardProps) {
  const colors = categoryColors[tip.category] || defaultColor;

  return (
    <button
      onClick={onClick}
      className={cn(
        "flex-shrink-0 flex items-center gap-2 px-4 py-2.5",
        "rounded-full",
        "border backdrop-blur-sm",
        colors.bg,
        colors.border,
        "transition-all duration-300 ease-out",
        "hover:scale-110 hover:shadow-lg hover:z-10",
        "cursor-pointer whitespace-nowrap",
        "group"
      )}
    >
      <span className={cn("opacity-70 group-hover:opacity-100 transition-opacity", colors.text)}>
        {tip.icon}
      </span>
      <span className="text-sm text-text-default group-hover:text-text-default dark:text-white/80 dark:group-hover:text-white transition-colors">
        {tip.text}
      </span>
    </button>
  );
}

interface ScrollingRowProps {
  tips: Tip[];
  direction: 'left' | 'right';
  speed: number; // seconds for one complete cycle
  onSelectTip: (prompt: string) => void;
}

function ScrollingRow({ tips, direction, speed, onSelectTip }: ScrollingRowProps) {
  const [isPaused, setIsPaused] = useState(false);

  // Duplicate tips for seamless loop
  const duplicatedTips = [...tips, ...tips];

  return (
    <div
      className="relative overflow-hidden py-2"
      onMouseEnter={() => setIsPaused(true)}
      onMouseLeave={() => setIsPaused(false)}
    >
      <div
        className={cn(
          "flex gap-3",
          direction === 'left' ? 'animate-scroll-left' : 'animate-scroll-right'
        )}
        style={{
          animationDuration: `${speed}s`,
          animationPlayState: isPaused ? 'paused' : 'running',
        }}
      >
        {duplicatedTips.map((tip, index) => (
          <TipCard
            key={`${tip.text}-${index}`}
            tip={tip}
            onClick={() => onSelectTip(tip.prompt)}
          />
        ))}
      </div>
    </div>
  );
}

interface ScrollingTipsGridProps {
  onSelectTip: (prompt: string) => void;
}

export function ScrollingTipsGrid({ onSelectTip }: ScrollingTipsGridProps) {
  // Split tips into 4 rows, memoized to prevent re-shuffle on re-render
  const rows = useMemo(() => splitIntoRows(tips, 4), []);

  // Row configurations: direction and speed
  const rowConfigs = [
    { direction: 'left' as const, speed: 45 },   // Row 1: slow
    { direction: 'right' as const, speed: 40 },  // Row 2: medium
    { direction: 'left' as const, speed: 50 },   // Row 3: slower
    { direction: 'right' as const, speed: 35 },  // Row 4: faster
  ];

  return (
    <div className="w-full h-full flex flex-col justify-center overflow-hidden">
      {/* Gradient overlay - left */}
      <div className="pointer-events-none absolute left-0 top-0 bottom-0 w-24 bg-gradient-to-r from-background-default to-transparent z-10" />

      {/* Gradient overlay - right */}
      <div className="pointer-events-none absolute right-0 top-0 bottom-0 w-24 bg-gradient-to-l from-background-default to-transparent z-10" />

      {/* Scrolling rows */}
      <div className="flex flex-col gap-2 py-4">
        {rows.map((rowTips, index) => (
          <ScrollingRow
            key={index}
            tips={rowTips}
            direction={rowConfigs[index].direction}
            speed={rowConfigs[index].speed}
            onSelectTip={onSelectTip}
          />
        ))}
      </div>

      {/* Subtle hint */}
      <div className="text-center mt-4">
        <span className="text-xs text-text-muted/50">
          点击任意提示开始对话
        </span>
      </div>
    </div>
  );
}
