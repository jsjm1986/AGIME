import { useState, useEffect } from 'react';

interface AnimatedAgimeLogoProps {
  className?: string;
  variant?: 'pulse' | 'rotate' | 'breathe';
  speed?: 'slow' | 'normal' | 'fast';
}

// Speed constants - defined outside component to avoid recreation on every render
const SPEEDS = {
  slow: 400,
  normal: 250,
  fast: 150,
};

/**
 * AnimatedAgimeLogo - Animated version of AGIME logo
 * Design: Two interlocking rings (AI + Me) with pulsing connection point
 *
 * Variants:
 * - pulse: Rings alternate pulsing, connection point glows
 * - rotate: Subtle rotation animation
 * - breathe: Gentle breathing/scaling effect
 */
export default function AnimatedAgimeLogo({
  className = '',
  variant = 'pulse',
  speed = 'normal',
}: AnimatedAgimeLogoProps) {
  const [frame, setFrame] = useState(0);

  useEffect(() => {
    const interval = setInterval(() => {
      setFrame((prev) => (prev + 1) % 8);
    }, SPEEDS[speed]);

    return () => clearInterval(interval);
  }, [speed]);

  // Animation states based on frame
  const aiRingOpacity = variant === 'pulse' ? (frame < 4 ? 1 : 0.5) : 1;
  const meRingOpacity = variant === 'pulse' ? (frame >= 4 ? 1 : 0.5) : 1;
  const centerScale = 0.8 + (Math.sin(frame * Math.PI / 4) * 0.2);
  const centerOpacity = 0.6 + (Math.sin(frame * Math.PI / 4) * 0.4);

  // Rotation for rotate variant
  const rotation = variant === 'rotate' ? frame * 45 : 0;

  // Scale for breathe variant
  const breatheScale = variant === 'breathe' ? 0.9 + (Math.sin(frame * Math.PI / 4) * 0.1) : 1;

  return (
    <div className={`w-4 h-4 ${className}`}>
      <svg
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        style={{
          transform: `rotate(${rotation}deg) scale(${breatheScale})`,
          transition: 'transform 0.15s ease-out',
        }}
      >
        {/* AI Ring - Left circle */}
        <path
          d="M9 5.5a5.5 5.5 0 1 0 3.5 9.74"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          style={{
            opacity: aiRingOpacity,
            transition: 'opacity 0.15s ease-out',
          }}
        />
        {/* Me Ring - Right circle */}
        <path
          d="M15 18.5a5.5 5.5 0 1 0-3.5-9.74"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          style={{
            opacity: meRingOpacity,
            transition: 'opacity 0.15s ease-out',
          }}
        />
        {/* Connection point - outer glow */}
        <circle
          cx="12"
          cy="12"
          r="2.5"
          fill="currentColor"
          style={{
            opacity: centerOpacity * 0.3,
            transform: `scale(${centerScale})`,
            transformOrigin: '12px 12px',
            transition: 'all 0.15s ease-out',
          }}
        />
        {/* Connection point - inner core */}
        <circle
          cx="12"
          cy="12"
          r="1.5"
          fill="currentColor"
          style={{
            opacity: centerOpacity,
            transition: 'opacity 0.15s ease-out',
          }}
        />
      </svg>
    </div>
  );
}

/**
 * Pre-configured variants for different states
 */
export function AgimeThinking({ className = '' }: { className?: string }) {
  return <AnimatedAgimeLogo className={className} variant="pulse" speed="normal" />;
}

export function AgimeWorking({ className = '' }: { className?: string }) {
  return <AnimatedAgimeLogo className={className} variant="rotate" speed="fast" />;
}

export function AgimeWaiting({ className = '' }: { className?: string }) {
  return <AnimatedAgimeLogo className={className} variant="breathe" speed="slow" />;
}
