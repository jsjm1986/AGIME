import React, { useEffect } from 'react';
import { FileText, Clock, Home, Puzzle, History, MessageCirclePlus } from 'lucide-react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  SidebarContent,
  SidebarFooter,
  SidebarMenu,
  SidebarMenuItem,
  SidebarMenuButton,
  SidebarGroup,
  SidebarGroupContent,
  SidebarSeparator,
} from '../ui/sidebar';
import { ChatSmart, Gear } from '../icons';
import { ViewOptions, View } from '../../utils/navigationUtils';
import { useChatContext } from '../../contexts/ChatContext';
import { DEFAULT_CHAT_TITLE } from '../../contexts/ChatContext';
import EnvironmentBadge from './EnvironmentBadge';
import ThemeToggleButton from './ThemeToggleButton';

interface SidebarProps {
  onSelectSession: (sessionId: string) => void;
  refreshTrigger?: number;
  children?: React.ReactNode;
  setView?: (view: View, viewOptions?: ViewOptions) => void;
  currentPath?: string;
}

interface NavigationItem {
  type: 'item';
  path: string;
  labelKey: string;
  tooltipKey: string;
  icon: React.ComponentType<{ className?: string }>;
}

interface NavigationAction {
  type: 'action';
  action: 'newChat' | 'currentChat';
  labelKey: string;
  tooltipKey: string;
  icon: React.ComponentType<{ className?: string }>;
}

interface NavigationSeparator {
  type: 'separator';
}

type NavigationEntry = NavigationItem | NavigationAction | NavigationSeparator;

const menuItemsConfig: NavigationEntry[] = [
  {
    type: 'item',
    path: '/',
    labelKey: 'home',
    tooltipKey: 'tooltips.home',
    icon: Home,
  },
  { type: 'separator' },
  {
    type: 'action',
    action: 'newChat',
    labelKey: 'newChat',
    tooltipKey: 'tooltips.newChat',
    icon: MessageCirclePlus,
  },
  {
    type: 'action',
    action: 'currentChat',
    labelKey: 'currentChat',
    tooltipKey: 'tooltips.currentChat',
    icon: ChatSmart,
  },
  {
    type: 'item',
    path: '/sessions',
    labelKey: 'history',
    tooltipKey: 'tooltips.history',
    icon: History,
  },
  { type: 'separator' },
  {
    type: 'item',
    path: '/recipes',
    labelKey: 'recipes',
    tooltipKey: 'tooltips.recipes',
    icon: FileText,
  },
  {
    type: 'item',
    path: '/schedules',
    labelKey: 'scheduler',
    tooltipKey: 'tooltips.scheduler',
    icon: Clock,
  },
  {
    type: 'item',
    path: '/extensions',
    labelKey: 'extensions',
    tooltipKey: 'tooltips.extensions',
    icon: Puzzle,
  },
  { type: 'separator' },
  {
    type: 'item',
    path: '/settings',
    labelKey: 'settings',
    tooltipKey: 'tooltips.settings',
    icon: Gear,
  },
];

const AppSidebar: React.FC<SidebarProps> = ({ currentPath }) => {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const chatContext = useChatContext();
  const { t } = useTranslation('sidebar');

  // Check if we're viewing an existing session (has resumeSessionId in URL)
  const resumeSessionId = searchParams.get('resumeSessionId');

  useEffect(() => {
    const timer = setTimeout(() => {
      // setIsVisible(true);
    }, 100);

    return () => clearTimeout(timer);
  }, []);

  useEffect(() => {
    const currentItem = menuItemsConfig.find(
      (item) => item.type === 'item' && item.path === currentPath
    ) as NavigationItem | undefined;

    const titleBits = ['AGIME'];

    if (
      currentPath === '/pair' &&
      chatContext?.chat?.name &&
      chatContext.chat.name !== DEFAULT_CHAT_TITLE
    ) {
      titleBits.push(chatContext.chat.name);
    } else if (currentPath !== '/' && currentItem) {
      titleBits.push(t(currentItem.labelKey));
    }

    document.title = titleBits.join(' - ');
  }, [currentPath, chatContext?.chat?.name, t]);

  const isActivePath = (path: string) => {
    return currentPath === path;
  };

  // Check if there's an active session that can be resumed
  // Use activeSessionId which persists across page navigation
  const hasActiveSession = !!chatContext?.activeSessionId;

  // Handle action clicks
  const handleActionClick = (action: 'newChat' | 'currentChat') => {
    if (action === 'newChat') {
      // Navigate to Pair with isNewSession flag to show OrbitTips
      navigate('/pair', {
        state: { isNewSession: true }
      });
    } else if (action === 'currentChat') {
      // Resume the current/last active session using activeSessionId
      if (chatContext?.activeSessionId) {
        navigate('/pair', {
          state: { resumeSessionId: chatContext.activeSessionId }
        });
      } else {
        // No active session, go to new chat
        navigate('/pair', {
          state: { isNewSession: true }
        });
      }
    }
  };

  const renderMenuItem = (entry: NavigationEntry, index: number) => {
    if (entry.type === 'separator') {
      return <SidebarSeparator key={index} />;
    }

    if (entry.type === 'action') {
      const IconComponent = entry.icon;
      const label = t(entry.labelKey);
      const tooltip = t(entry.tooltipKey);
      const isCurrentChatDisabled = entry.action === 'currentChat' && !hasActiveSession;
      // "当前对话" should only be highlighted when:
      // 1. We're on /pair route AND
      // 2. We're viewing an existing session (has resumeSessionId in URL)
      // New chat page (no resumeSessionId) should NOT highlight "当前对话"
      const isActive = entry.action === 'currentChat' && currentPath === '/pair' && !!resumeSessionId;

      return (
        <SidebarGroup key={entry.action}>
          <SidebarGroupContent className="space-y-0.5">
            <div className="sidebar-item">
              <SidebarMenuItem>
                <SidebarMenuButton
                  data-testid={`sidebar-${entry.labelKey}-button`}
                  onClick={() => handleActionClick(entry.action)}
                  isActive={isActive}
                  tooltip={isCurrentChatDisabled ? t('tooltips.noActiveChat') : tooltip}
                  disabled={isCurrentChatDisabled}
                  className={`w-full justify-start px-3 rounded-md h-9 transition-all duration-100 ${
                    isCurrentChatDisabled
                      ? 'text-text-muted/40 dark:text-white/20 cursor-not-allowed'
                      : isActive
                        ? 'bg-black/5 text-text-default dark:bg-white/10 dark:text-white'
                        : 'text-text-muted hover:text-text-default hover:bg-black/5 dark:text-white/50 dark:hover:text-white/80 dark:hover:bg-white/5'
                  }`}
                >
                  <IconComponent className="w-4 h-4" />
                  <span className="font-medium">{label}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            </div>
          </SidebarGroupContent>
        </SidebarGroup>
      );
    }

    const IconComponent = entry.icon;
    const label = t(entry.labelKey);
    const tooltip = t(entry.tooltipKey);
    const isActive = isActivePath(entry.path);

    return (
      <SidebarGroup key={entry.path}>
        <SidebarGroupContent className="space-y-0.5">
          <div className="sidebar-item">
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid={`sidebar-${entry.labelKey}-button`}
                onClick={() => navigate(entry.path)}
                isActive={isActive}
                tooltip={tooltip}
                className={`w-full justify-start px-3 rounded-md h-9 transition-all duration-100 ${
                  isActive
                    ? 'bg-black/5 text-text-default dark:bg-white/10 dark:text-white'
                    : 'text-text-muted hover:text-text-default hover:bg-black/5 dark:text-white/50 dark:hover:text-white/80 dark:hover:bg-white/5'
                }`}
              >
                <IconComponent className="w-4 h-4" />
                <span className="font-medium">{label}</span>
              </SidebarMenuButton>
            </SidebarMenuItem>
          </div>
        </SidebarGroupContent>
      </SidebarGroup>
    );
  };

  return (
    <>
      <SidebarContent className="pt-16">
        <SidebarMenu>{menuItemsConfig.map((entry, index) => renderMenuItem(entry, index))}</SidebarMenu>
      </SidebarContent>

      <SidebarFooter className="pb-2 flex flex-col items-start gap-2">
        <div className="flex items-center justify-between w-full px-2">
          <EnvironmentBadge />
          <ThemeToggleButton />
        </div>
      </SidebarFooter>
    </>
  );
};

export default AppSidebar;
