import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { cn } from '../../utils';
import { Tip } from './ScrollingTips';

interface TipSphereProps {
  tips: Tip[];
  onSelectTip: (prompt: string) => void;
}

// 分类颜色映射 - 丰富的色彩系统
const categoryColors: Record<string, { bg: string; text: string; glow: string }> = {
  '清理': { bg: 'bg-emerald-100 dark:bg-emerald-500/25', text: 'text-emerald-700 dark:text-emerald-300', glow: 'shadow-emerald-500/40' },
  '系统': { bg: 'bg-blue-100 dark:bg-blue-500/25', text: 'text-blue-700 dark:text-blue-300', glow: 'shadow-blue-500/40' },
  '整理': { bg: 'bg-amber-100 dark:bg-amber-500/25', text: 'text-amber-700 dark:text-amber-300', glow: 'shadow-amber-500/40' },
  '记忆': { bg: 'bg-purple-100 dark:bg-purple-500/25', text: 'text-purple-700 dark:text-purple-300', glow: 'shadow-purple-500/40' },
  '文档': { bg: 'bg-cyan-100 dark:bg-cyan-500/25', text: 'text-cyan-700 dark:text-cyan-300', glow: 'shadow-cyan-500/40' },
  '数据': { bg: 'bg-pink-100 dark:bg-pink-500/25', text: 'text-pink-700 dark:text-pink-300', glow: 'shadow-pink-500/40' },
  '截图': { bg: 'bg-orange-100 dark:bg-orange-500/25', text: 'text-orange-700 dark:text-orange-300', glow: 'shadow-orange-500/40' },
  '办公': { bg: 'bg-indigo-100 dark:bg-indigo-500/25', text: 'text-indigo-700 dark:text-indigo-300', glow: 'shadow-indigo-500/40' },
  '生活': { bg: 'bg-rose-100 dark:bg-rose-500/25', text: 'text-rose-700 dark:text-rose-300', glow: 'shadow-rose-500/40' },
  '社交': { bg: 'bg-violet-100 dark:bg-violet-500/25', text: 'text-violet-700 dark:text-violet-300', glow: 'shadow-violet-500/40' },
  '学习': { bg: 'bg-teal-100 dark:bg-teal-500/25', text: 'text-teal-700 dark:text-teal-300', glow: 'shadow-teal-500/40' },
  '自动化': { bg: 'bg-yellow-100 dark:bg-yellow-500/25', text: 'text-yellow-700 dark:text-yellow-300', glow: 'shadow-yellow-500/40' },
  '创意': { bg: 'bg-fuchsia-100 dark:bg-fuchsia-500/25', text: 'text-fuchsia-700 dark:text-fuchsia-300', glow: 'shadow-fuchsia-500/40' },
  '搜索': { bg: 'bg-lime-100 dark:bg-lime-500/25', text: 'text-lime-700 dark:text-lime-300', glow: 'shadow-lime-500/40' },
  '探索': { bg: 'bg-sky-100 dark:bg-sky-500/25', text: 'text-sky-700 dark:text-sky-300', glow: 'shadow-sky-500/40' },
};

const defaultColor = { bg: 'from-block-teal/30 to-block-teal/20', text: 'text-block-teal', glow: 'shadow-block-teal/40' };

// 简化的提示数据
interface SphereTip {
  text: string;
  prompt: string;
  category: string;
}

export function TipSphere({ tips, onSelectTip }: TipSphereProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const sphereRef = useRef<HTMLDivElement>(null);
  const [rotation, setRotation] = useState({ x: -10, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const [isHoveringBall, setIsHoveringBall] = useState(false);
  const [containerSize, setContainerSize] = useState({ width: 600, height: 500 });
  const dragStartRef = useRef({ x: 0, y: 0, rotX: 0, rotY: 0 });

  // 响应式计算球体半径 - 根据容器大小自适应
  const radius = useMemo(() => {
    const minDimension = Math.min(containerSize.width, containerSize.height);
    // 球体半径为容器较小边的 35%
    return Math.max(120, Math.min(220, minDimension * 0.35));
  }, [containerSize]);

  // 监听容器尺寸变化
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

  // 转换提示数据
  const sphereTips: SphereTip[] = useMemo(() =>
    tips.map(tip => ({
      text: tip.text,
      prompt: tip.prompt,
      category: tip.category
    })),
    [tips]
  );

  // 斐波那契球分布算法
  const calculatePositions = useCallback((total: number, r: number) => {
    const positions: { x: number; y: number; z: number }[] = [];
    const goldenRatio = (1 + Math.sqrt(5)) / 2;

    for (let i = 0; i < total; i++) {
      const theta = 2 * Math.PI * i / goldenRatio;
      const phi = Math.acos(1 - 2 * (i + 0.5) / total);

      const x = Math.sin(phi) * Math.cos(theta);
      const y = Math.sin(phi) * Math.sin(theta);
      const z = Math.cos(phi);

      positions.push({ x: x * r, y: y * r, z: z * r });
    }

    return positions;
  }, []);

  const positions = useMemo(
    () => calculatePositions(sphereTips.length, radius),
    [calculatePositions, sphereTips.length, radius]
  );

  // 自动旋转 - 只有在不拖拽且鼠标不在球上时才旋转
  useEffect(() => {
    if (isHoveringBall || isDragging) return;

    const interval = setInterval(() => {
      setRotation(prev => ({
        ...prev,
        y: prev.y + 0.15
      }));
    }, 30);

    return () => clearInterval(interval);
  }, [isHoveringBall, isDragging]);

  // 鼠标拖拽
  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
    dragStartRef.current = {
      x: e.clientX,
      y: e.clientY,
      rotX: rotation.x,
      rotY: rotation.y
    };
  }, [rotation]);

  const handleMouseMove = useCallback((e: MouseEvent) => {
    if (!isDragging) return;

    const deltaX = e.clientX - dragStartRef.current.x;
    const deltaY = e.clientY - dragStartRef.current.y;

    setRotation({
      x: Math.max(-60, Math.min(60, dragStartRef.current.rotX + deltaY * 0.3)),
      y: dragStartRef.current.rotY + deltaX * 0.3
    });
  }, [isDragging]);

  const handleMouseUp = useCallback(() => {
    setIsDragging(false);
  }, []);

  useEffect(() => {
    if (isDragging) {
      window.addEventListener('mousemove', handleMouseMove);
      window.addEventListener('mouseup', handleMouseUp);
    }

    return () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDragging, handleMouseMove, handleMouseUp]);

  // 3D坐标转换
  const transformPoint = useCallback((x: number, y: number, z: number) => {
    const radX = (rotation.x * Math.PI) / 180;
    const radY = (rotation.y * Math.PI) / 180;

    // 绕Y轴旋转
    const x1 = x * Math.cos(radY) - z * Math.sin(radY);
    const z1 = x * Math.sin(radY) + z * Math.cos(radY);

    // 绕X轴旋转
    const y1 = y * Math.cos(radX) - z1 * Math.sin(radX);
    const z2 = y * Math.sin(radX) + z1 * Math.cos(radX);

    return { x: x1, y: y1, z: z2 };
  }, [rotation]);

  // 获取分类颜色
  const getColor = (category: string) => categoryColors[category] || defaultColor;

  return (
    <div
      ref={containerRef}
      className="w-full h-full flex items-center justify-center"
    >
      {/* 球体交互区域 */}
      <div
        ref={sphereRef}
        className={cn(
          "relative flex items-center justify-center",
          isDragging ? "cursor-grabbing" : "cursor-grab"
        )}
        style={{
          width: radius * 2 + 120,
          height: radius * 2 + 120,
        }}
        onMouseDown={handleMouseDown}
        onMouseEnter={() => setIsHoveringBall(true)}
        onMouseLeave={() => {
          setIsHoveringBall(false);
          setIsDragging(false);
        }}
      >
        {/* 中心发光效果 - 多层渐变 */}
        <div
          className="absolute rounded-full bg-gradient-radial from-block-teal/20 via-block-teal/5 to-transparent tip-sphere-center-glow"
          style={{
            width: radius * 1.5,
            height: radius * 1.5,
          }}
        />
        <div
          className="absolute rounded-full bg-gradient-radial from-block-orange/10 via-transparent to-transparent"
          style={{
            width: radius * 1.2,
            height: radius * 1.2,
          }}
        />

        {/* 提示项 */}
        {sphereTips.map((tip, index) => {
          const pos = positions[index];
          if (!pos) return null;

          const transformed = transformPoint(pos.x, pos.y, pos.z);
          const normalizedZ = (transformed.z + radius) / (2 * radius);
          const opacity = 0.2 + normalizedZ * 0.8;
          const scale = 0.6 + normalizedZ * 0.4;
          const zIndex = Math.round(normalizedZ * 100);
          const color = getColor(tip.category);

          return (
            <button
              key={index}
              className={cn(
                "absolute px-3 py-1.5 rounded-full text-xs font-medium",
                "bg-gradient-to-br backdrop-blur-sm",
                "border border-black/10 dark:border-white/20",
                "whitespace-nowrap",
                "transition-all duration-200 ease-out",
                "hover:scale-125 hover:z-[200] hover:border-black/20 dark:hover:border-white/40",
                "focus:outline-none focus:ring-2 focus:ring-black/20 dark:focus:ring-white/30",
                color.bg,
                color.text
              )}
              style={{
                left: '50%',
                top: '50%',
                transform: `translate(calc(-50% + ${transformed.x}px), calc(-50% + ${transformed.y}px)) scale(${scale})`,
                opacity,
                zIndex,
                pointerEvents: normalizedZ > 0.25 ? 'auto' : 'none',
                boxShadow: normalizedZ > 0.6 ? `0 0 20px var(--tw-shadow-color)` : 'none',
              }}
              onClick={(e) => {
                e.stopPropagation();
                onSelectTip(tip.prompt);
              }}
              onMouseDown={(e) => e.stopPropagation()}
            >
              <span className="flex items-center gap-1.5">
                <span className={cn(
                  "text-[10px] px-1.5 py-0.5 rounded-full",
                  "bg-black/5 dark:bg-white/10 backdrop-blur-sm"
                )}>
                  {tip.category}
                </span>
                <span>{tip.text}</span>
              </span>
            </button>
          );
        })}
      </div>

      {/* 操作提示 */}
      <div className="absolute bottom-8 left-1/2 -translate-x-1/2 text-xs text-text-muted/60 flex items-center gap-2">
        <span className="px-2 py-1 rounded-full bg-black/5 dark:bg-white/5 backdrop-blur-sm">
          拖拽旋转
        </span>
        <span className="text-text-muted/30">·</span>
        <span className="px-2 py-1 rounded-full bg-black/5 dark:bg-white/5 backdrop-blur-sm">
          点击选择
        </span>
      </div>
    </div>
  );
}
