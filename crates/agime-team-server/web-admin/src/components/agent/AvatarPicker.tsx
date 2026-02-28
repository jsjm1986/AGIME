import { useState, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Bot, Brain, Code, Cpu, Globe, Lightbulb, MessageSquare, Rocket,
  Search, Shield, Sparkles, Terminal, Wrench, Zap, BookOpen, Eye,
  type LucideIcon,
} from 'lucide-react';
import agimeLogo from '../../assets/agime-logo.svg';

export const AGENT_AVATAR_ICONS: Record<string, LucideIcon> = {
  bot: Bot,
  brain: Brain,
  code: Code,
  cpu: Cpu,
  globe: Globe,
  lightbulb: Lightbulb,
  message: MessageSquare,
  rocket: Rocket,
  search: Search,
  shield: Shield,
  sparkles: Sparkles,
  terminal: Terminal,
  wrench: Wrench,
  zap: Zap,
  book: BookOpen,
  eye: Eye,
};

interface AgentAvatarProps {
  avatar?: string;
  name: string;
  className?: string;
  iconSize?: string;
}

export function AgentAvatar({ avatar, name, className = 'w-8 h-8 bg-muted', iconSize = 'w-4 h-4' }: AgentAvatarProps) {
  const Icon = avatar ? AGENT_AVATAR_ICONS[avatar] : undefined;
  return (
    <div className={`${className} rounded-full flex items-center justify-center shrink-0`}>
      {Icon ? (
        <Icon className={iconSize} />
      ) : (
        <img src={agimeLogo} alt={name} className={iconSize} />
      )}
    </div>
  );
}

interface AvatarPickerProps {
  value?: string;
  onChange: (value?: string) => void;
}

export function AvatarPicker({ value, onChange }: AvatarPickerProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  const icons = Object.entries(AGENT_AVATAR_ICONS);
  const CurrentIcon = value ? AGENT_AVATAR_ICONS[value] : undefined;

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 px-3 py-1.5 rounded-md border border-border hover:bg-muted transition-colors"
      >
        <div className="w-7 h-7 rounded-full bg-muted flex items-center justify-center">
          {CurrentIcon ? (
            <CurrentIcon className="w-4 h-4" />
          ) : (
            <img src={agimeLogo} alt="default" className="w-4 h-4" />
          )}
        </div>
        <span className="text-sm text-muted-foreground">
          {value ? t('agent.avatar.change') : t('agent.avatar.select')}
        </span>
      </button>

      {open && (
        <div className="absolute z-50 mt-1 p-2 rounded-lg border border-border bg-popover shadow-md w-[280px]">
          <p className="text-xs text-muted-foreground mb-2">{t('agent.avatar.pickIcon')}</p>
          <div className="grid grid-cols-8 gap-1">
            {/* Default = brand logo */}
            <button
              type="button"
              onClick={() => { onChange(undefined); setOpen(false); }}
              className={`w-8 h-8 rounded-md border flex items-center justify-center transition-colors ${
                !value ? 'border-primary bg-primary/10' : 'border-transparent hover:bg-muted'
              }`}
              title={t('agent.avatar.default')}
            >
              <img src={agimeLogo} alt="default" className="w-4 h-4" />
            </button>
            {icons.map(([key, Icon]) => (
              <button
                key={key}
                type="button"
                onClick={() => { onChange(key); setOpen(false); }}
                className={`w-8 h-8 rounded-md border flex items-center justify-center transition-colors ${
                  value === key ? 'border-primary bg-primary/10 text-primary' : 'border-transparent hover:bg-muted text-muted-foreground'
                }`}
                title={key}
              >
                <Icon className="w-4 h-4" />
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
