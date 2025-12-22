import { useState, useEffect, useMemo, useRef } from 'react';
import { cn } from '../../utils';
import { tips, Tip } from './ScrollingTips';
import { Goose } from '../icons/Goose';

interface OrbitTipsProps {
  onSelectTip: (prompt: string) => void;
}

// 轨道配置
interface OrbitConfig {
  radiusX: number;      // 椭圆X轴半径比例
  radiusY: number;      // 椭圆Y轴半径比例
  duration: number;     // 旋转周期（秒）
  direction: 1 | -1;    // 旋转方向
  tiltX: number;        // X轴倾斜角度
  items: number;        // 该轨道上的项目数
}

const orbitConfigs: OrbitConfig[] = [
  { radiusX: 0.22, radiusY: 0.18, duration: 25, direction: 1, tiltX: 65, items: 6 },
  { radiusX: 0.35, radiusY: 0.28, duration: 35, direction: -1, tiltX: 65, items: 8 },
  { radiusX: 0.48, radiusY: 0.38, duration: 45, direction: 1, tiltX: 65, items: 10 },
];

// 分类颜色
const categoryColors: Record<string, { bg: string; border: string; text: string; shadow: string }> = {
  '清理': { bg: 'bg-emerald-100 dark:bg-emerald-500/20', border: 'border-emerald-200 dark:border-emerald-400/40', text: 'text-emerald-700 dark:text-emerald-300', shadow: 'shadow-emerald-500/30' },
  '系统': { bg: 'bg-blue-100 dark:bg-blue-500/20', border: 'border-blue-200 dark:border-blue-400/40', text: 'text-blue-700 dark:text-blue-300', shadow: 'shadow-blue-500/30' },
  '整理': { bg: 'bg-amber-100 dark:bg-amber-500/20', border: 'border-amber-200 dark:border-amber-400/40', text: 'text-amber-700 dark:text-amber-300', shadow: 'shadow-amber-500/30' },
  '记忆': { bg: 'bg-purple-100 dark:bg-purple-500/20', border: 'border-purple-200 dark:border-purple-400/40', text: 'text-purple-700 dark:text-purple-300', shadow: 'shadow-purple-500/30' },
  '文档': { bg: 'bg-cyan-100 dark:bg-cyan-500/20', border: 'border-cyan-200 dark:border-cyan-400/40', text: 'text-cyan-700 dark:text-cyan-300', shadow: 'shadow-cyan-500/30' },
  '数据': { bg: 'bg-pink-100 dark:bg-pink-500/20', border: 'border-pink-200 dark:border-pink-400/40', text: 'text-pink-700 dark:text-pink-300', shadow: 'shadow-pink-500/30' },
  '截图': { bg: 'bg-orange-100 dark:bg-orange-500/20', border: 'border-orange-200 dark:border-orange-400/40', text: 'text-orange-700 dark:text-orange-300', shadow: 'shadow-orange-500/30' },
  '办公': { bg: 'bg-indigo-100 dark:bg-indigo-500/20', border: 'border-indigo-200 dark:border-indigo-400/40', text: 'text-indigo-700 dark:text-indigo-300', shadow: 'shadow-indigo-500/30' },
  '生活': { bg: 'bg-rose-100 dark:bg-rose-500/20', border: 'border-rose-200 dark:border-rose-400/40', text: 'text-rose-700 dark:text-rose-300', shadow: 'shadow-rose-500/30' },
  '社交': { bg: 'bg-violet-100 dark:bg-violet-500/20', border: 'border-violet-200 dark:border-violet-400/40', text: 'text-violet-700 dark:text-violet-300', shadow: 'shadow-violet-500/30' },
  '学习': { bg: 'bg-teal-100 dark:bg-teal-500/20', border: 'border-teal-200 dark:border-teal-400/40', text: 'text-teal-700 dark:text-teal-300', shadow: 'shadow-teal-500/30' },
  '自动化': { bg: 'bg-yellow-100 dark:bg-yellow-500/20', border: 'border-yellow-200 dark:border-yellow-400/40', text: 'text-yellow-700 dark:text-yellow-300', shadow: 'shadow-yellow-500/30' },
  '创意': { bg: 'bg-fuchsia-100 dark:bg-fuchsia-500/20', border: 'border-fuchsia-200 dark:border-fuchsia-400/40', text: 'text-fuchsia-700 dark:text-fuchsia-300', shadow: 'shadow-fuchsia-500/30' },
  '搜索': { bg: 'bg-lime-100 dark:bg-lime-500/20', border: 'border-lime-200 dark:border-lime-400/40', text: 'text-lime-700 dark:text-lime-300', shadow: 'shadow-lime-500/30' },
  '探索': { bg: 'bg-sky-100 dark:bg-sky-500/20', border: 'border-sky-200 dark:border-sky-400/40', text: 'text-sky-700 dark:text-sky-300', shadow: 'shadow-sky-500/30' },
};

const defaultColor = { bg: 'bg-block-teal/20', border: 'border-block-teal/40', text: 'text-block-teal', shadow: 'shadow-block-teal/30' };

export function OrbitTips({ onSelectTip }: OrbitTipsProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerSize, setContainerSize] = useState({ width: 800, height: 600 });
  const [pausedOrbits, setPausedOrbits] = useState<Set<number>>(new Set());
  const [time, setTime] = useState(0);

  // 监听容器尺寸
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const updateSize = () => {
      const rect = container.getBoundingClientRect();
      setContainerSize({ width: rect.width, height: rect.height });
    };

    updateSize();
    const resizeObserver = new ResizeObserver(updateSize);
    resizeObserver.observe(container);

    return () => resizeObserver.disconnect();
  }, []);

  // 动画时间
  useEffect(() => {
    const interval = setInterval(() => {
      setTime(t => t + 0.016); // ~60fps
    }, 16);
    return () => clearInterval(interval);
  }, []);

  // 随机打乱并分配提示到各轨道
  const orbitTips = useMemo(() => {
    const shuffled = [...tips].sort(() => Math.random() - 0.5);
    const result: Tip[][] = [];
    let index = 0;

    for (const config of orbitConfigs) {
      result.push(shuffled.slice(index, index + config.items));
      index += config.items;
    }

    return result;
  }, []);

  // 计算椭圆轨道上的位置
  const getPosition = (
    orbitIndex: number,
    itemIndex: number,
    totalItems: number,
    config: OrbitConfig
  ) => {
    const baseAngle = (itemIndex / totalItems) * Math.PI * 2;
    const isPaused = pausedOrbits.has(orbitIndex);
    const currentTime = isPaused ? 0 : time;
    const angle = baseAngle + (currentTime * config.direction * Math.PI * 2) / config.duration;

    const centerX = containerSize.width / 2;
    const centerY = containerSize.height / 2;
    const radiusX = containerSize.width * config.radiusX;
    const radiusY = containerSize.height * config.radiusY;

    // 椭圆轨道位置
    const x = centerX + radiusX * Math.cos(angle);
    const y = centerY + radiusY * Math.sin(angle) * Math.cos((config.tiltX * Math.PI) / 180);

    // 根据Y位置计算景深（模拟3D）
    const depth = Math.sin(angle);
    const scale = 0.7 + 0.3 * (1 + depth) / 2;
    const opacity = 0.4 + 0.6 * (1 + depth) / 2;
    const zIndex = Math.round(50 + depth * 50);

    return { x, y, scale, opacity, zIndex, depth };
  };

  const getColor = (category: string) => categoryColors[category] || defaultColor;

  return (
    <div
      ref={containerRef}
      className="w-full h-full relative overflow-hidden"
    >
      {/* 背景装饰 */}
      <div className="absolute inset-0 bg-gradient-radial from-block-teal/5 via-transparent to-transparent" />

      {/* 轨道线 */}
      {orbitConfigs.map((config, orbitIndex) => (
        <div
          key={`orbit-line-${orbitIndex}`}
          className="absolute border border-black/5 dark:border-white/5 rounded-full"
          style={{
            width: containerSize.width * config.radiusX * 2,
            height: containerSize.height * config.radiusY * 2 * Math.cos((config.tiltX * Math.PI) / 180),
            left: containerSize.width / 2 - containerSize.width * config.radiusX,
            top: containerSize.height / 2 - containerSize.height * config.radiusY * Math.cos((config.tiltX * Math.PI) / 180),
          }}
        />
      ))}

      {/* 中心 Logo */}
      <div
        className="absolute flex items-center justify-center"
        style={{
          left: containerSize.width / 2 - 40,
          top: containerSize.height / 2 - 40,
          width: 80,
          height: 80,
        }}
      >
        <div className="relative">
          {/* 发光效果 */}
          <div className="absolute inset-0 bg-block-teal/20 rounded-full blur-xl scale-150 animate-pulse" />
          <div className="absolute inset-0 bg-block-orange/10 rounded-full blur-2xl scale-200" />

          {/* Logo */}
          <div className="relative w-16 h-16 rounded-full bg-gradient-to-br from-block-teal/30 to-block-orange/20 backdrop-blur-sm border border-black/10 dark:border-white/20 flex items-center justify-center">
            <Goose className="w-8 h-8 text-text-default dark:text-white" />
          </div>
        </div>
      </div>

      {/* 轨道上的提示 */}
      {orbitConfigs.map((config, orbitIndex) => (
        <div
          key={`orbit-${orbitIndex}`}
          className="absolute inset-0"
          onMouseEnter={() => setPausedOrbits(prev => new Set(prev).add(orbitIndex))}
          onMouseLeave={() => setPausedOrbits(prev => {
            const next = new Set(prev);
            next.delete(orbitIndex);
            return next;
          })}
        >
          {orbitTips[orbitIndex]?.map((tip, itemIndex) => {
            const totalItems = orbitTips[orbitIndex].length;
            const pos = getPosition(orbitIndex, itemIndex, totalItems, config);
            const color = getColor(tip.category);

            return (
              <button
                key={`${orbitIndex}-${itemIndex}`}
                className={cn(
                  "absolute px-3 py-1.5 rounded-full",
                  "backdrop-blur-sm border",
                  "text-xs font-medium whitespace-nowrap",
                  "transition-all duration-300 ease-out",
                  "hover:scale-110 hover:z-[200]",
                  "focus:outline-none focus:ring-2 focus:ring-white/30",
                  color.bg,
                  color.border,
                  color.text
                )}
                style={{
                  left: pos.x,
                  top: pos.y,
                  transform: `translate(-50%, -50%) scale(${pos.scale})`,
                  opacity: pos.opacity,
                  zIndex: pos.zIndex,
                  boxShadow: pos.depth > 0.3 ? `0 0 20px var(--tw-shadow-color)` : 'none',
                }}
                onClick={() => onSelectTip(tip.prompt)}
              >
                <span className="flex items-center gap-1.5">
                  <span className="opacity-70">{tip.text}</span>
                </span>
              </button>
            );
          })}
        </div>
      ))}

      {/* 底部提示 */}
      <div className="absolute bottom-6 left-1/2 -translate-x-1/2 flex items-center gap-3 text-xs text-text-muted/50">
        <span className="px-3 py-1 rounded-full bg-black/5 dark:bg-white/5 backdrop-blur-sm">
          悬停暂停
        </span>
        <span>·</span>
        <span className="px-3 py-1 rounded-full bg-black/5 dark:bg-white/5 backdrop-blur-sm">
          点击选择
        </span>
      </div>
    </div>
  );
}
