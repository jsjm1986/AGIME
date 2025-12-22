import { AgimeLogo as AgimeLogoIcon, Rain } from './icons/Goose';
import { cn } from '../utils';

interface AgimeLogoProps {
  className?: string;
  size?: 'default' | 'small';
  hover?: boolean;
}

export type { AgimeLogoProps };

export default function AgimeLogo({
  className = '',
  size = 'default',
  hover = true,
}: AgimeLogoProps) {
  const sizes = {
    default: {
      frame: 'w-16 h-16',
      rain: 'w-[275px] h-[275px]',
      logo: 'w-16 h-16',
    },
    small: {
      frame: 'w-8 h-8',
      rain: 'w-[150px] h-[150px]',
      logo: 'w-8 h-8',
    },
  } as const;

  const currentSize = sizes[size];

  return (
    <div
      className={cn(
        className,
        currentSize.frame,
        'relative overflow-hidden',
        hover && 'group/with-hover'
      )}
    >
      <Rain
        className={cn(
          currentSize.rain,
          'absolute left-0 bottom-0 transition-all duration-300 z-1',
          hover && 'opacity-0 group-hover/with-hover:opacity-100'
        )}
      />
      <AgimeLogoIcon className={cn(currentSize.logo, 'absolute left-0 bottom-0 z-2')} />
    </div>
  );
}

// Backward compatibility - export type alias
export type GooseLogoProps = AgimeLogoProps;
