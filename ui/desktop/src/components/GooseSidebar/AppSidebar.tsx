import React, { useEffect } from 'react';
import { FileText, Clock, Home, Puzzle, History } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
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

interface NavigationSeparator {
  type: 'separator';
}

type NavigationEntry = NavigationItem | NavigationSeparator;

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
    type: 'item',
    path: '/pair',
    labelKey: 'chat',
    tooltipKey: 'tooltips.chat',
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
  const chatContext = useChatContext();
  const { t } = useTranslation('sidebar');

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

  const renderMenuItem = (entry: NavigationEntry, index: number) => {
    if (entry.type === 'separator') {
      return <SidebarSeparator key={index} />;
    }

    const IconComponent = entry.icon;
    const label = t(entry.labelKey);
    const tooltip = t(entry.tooltipKey);

    return (
      <SidebarGroup key={entry.path}>
        <SidebarGroupContent className="space-y-1">
          <div className="sidebar-item">
            <SidebarMenuItem>
              <SidebarMenuButton
                data-testid={`sidebar-${entry.labelKey}-button`}
                onClick={() => navigate(entry.path)}
                isActive={isActivePath(entry.path)}
                tooltip={tooltip}
                className="w-full justify-start px-3 rounded-lg h-fit hover:bg-background-medium/50 transition-all duration-200 data-[active=true]:bg-background-medium"
              >
                <IconComponent className="w-4 h-4" />
                <span>{label}</span>
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

      <SidebarFooter className="pb-2 flex items-start">
        <EnvironmentBadge />
      </SidebarFooter>
    </>
  );
};

export default AppSidebar;
