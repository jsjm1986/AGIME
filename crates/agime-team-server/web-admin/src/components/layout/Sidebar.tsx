import { Link, useLocation } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { useAuth } from '../../contexts/AuthContext';
import { useTeamContext } from '../../contexts/TeamContext';
import { ThemeToggle } from '../ThemeToggle';
import { LanguageSwitcher } from '../LanguageSwitcher';
import { Button } from '../ui/button';
import {
  UserPlus, ArrowLeft, Zap, FileText,
  Bot, MessageCircle, Users, ScrollText, Handshake, UserRound,
  PanelLeftClose, PanelLeftOpen, Globe, Github, ShieldCheck,
} from 'lucide-react';
import { NAV_ITEMS } from '../../config/teamNavConfig';
import { useBrand } from '../../contexts/BrandContext';
import agimeLogoSvg from '../../assets/agime-logo.svg';

const NAV_ICONS: Record<string, React.ReactNode> = {
  MessageCircle: <MessageCircle className="w-4 h-4" />,
  Bot: <Bot className="w-4 h-4" />,
  FileText: <FileText className="w-4 h-4" />,
  Zap: <Zap className="w-4 h-4" />,
  ScrollText: <ScrollText className="w-4 h-4" />,
  Handshake: <Handshake className="w-4 h-4" />,
  UserRound: <UserRound className="w-4 h-4" />,
  Globe: <Globe className="w-4 h-4" />,
  Users: <Users className="w-4 h-4" />,
};

/** Keys that show a count badge */
function getNavCount(
  key: string,
  team: { skillsCount?: number; membersCount?: number },
): number | null {
  switch (key) {
    case 'toolkit': return team.skillsCount ?? 0;
    case 'team-admin': return team.membersCount ?? 0;
    default: return null;
  }
}

/** Keys after which a visual separator is rendered */
const SEPARATOR_AFTER = new Set(['chat', 'smart-log', 'external-users']);

interface NavItem {
  path: string;
  labelKey: string;
  icon: React.ReactNode;
  adminOnly?: boolean;
}

const defaultNavItems: NavItem[] = [
  {
    path: '/dashboard',
    labelKey: 'sidebar.dashboard',
    icon: (
      <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
          d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6" />
      </svg>
    ),
  },
  {
    path: '/teams',
    labelKey: 'sidebar.teams',
    icon: (
      <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
          d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z" />
      </svg>
    ),
  },
  {
    path: '/api-keys',
    labelKey: 'sidebar.apiKeys',
    icon: (
      <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
          d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z" />
      </svg>
    ),
  },
  {
    path: '/settings',
    labelKey: 'sidebar.settings',
    icon: (
      <svg className="w-5 h-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2}
          d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    ),
  },
  {
    path: '/registrations',
    labelKey: 'sidebar.registrations',
    icon: <ShieldCheck className="w-5 h-5" />,
    adminOnly: true,
  },
];

interface SidebarProps {
  onNavigate?: () => void;
}

export function Sidebar({ onNavigate }: SidebarProps = {}) {
  const { t } = useTranslation();
  const { brand } = useBrand();
  const { user, logout, isAdmin } = useAuth();
  const location = useLocation();
  const teamCtx = useTeamContext();

  const collapsed = teamCtx?.sidebarCollapsed ?? false;

  const handleLogout = async () => {
    await logout();
  };

  // ── Collapsed team nav (icon rail) ──
  const renderCollapsedTeamNav = () => {
    if (!teamCtx) return null;
    const { team, activeSection, onSectionChange } = teamCtx;

    return (
      <>
        {/* Team avatar */}
        <div className="flex justify-center py-3 border-b border-[hsl(var(--sidebar-border))]">
          <div className="w-8 h-8 rounded-lg bg-[hsl(var(--primary))] flex items-center justify-center">
            <span className="text-white font-bold text-sm">
              {team.name.charAt(0).toUpperCase()}
            </span>
          </div>
        </div>

        {/* Icon nav */}
        <nav className="flex-1 overflow-y-auto py-2 flex flex-col items-center gap-0.5">
          {NAV_ITEMS.filter((item) => !item.adminOnly || teamCtx.canManage).map((item) => {
            const isActive = activeSection === item.key;
            const icon = NAV_ICONS[item.icon];
            return (
              <div key={item.key}>
                <button
                  type="button"
                  onClick={() => { onSectionChange(item.key); onNavigate?.(); }}
                  title={t(item.labelKey)}
                  className={`w-9 h-9 flex items-center justify-center rounded-md transition-colors ${
                    isActive
                      ? 'bg-[hsl(var(--sidebar-accent))] text-[hsl(var(--sidebar-accent-foreground))]'
                      : 'text-[hsl(var(--sidebar-foreground))] hover:bg-[hsl(var(--sidebar-accent))/0.5]'
                  }`}
                >
                  <span className={isActive ? 'opacity-100' : 'opacity-50'}>{icon}</span>
                </button>
                {SEPARATOR_AFTER.has(item.key) && (
                  <div className="my-1.5 mx-auto w-5 border-t border-[hsl(var(--sidebar-border))]" />
                )}
              </div>
            );
          })}
        </nav>
      </>
    );
  };

  // ── Full team nav ──
  const renderTeamNav = () => {
    if (!teamCtx) return null;
    const { team, canManage, activeSection, onSectionChange, onInviteClick } = teamCtx;

    return (
      <>
        {/* Team header */}
        <div className="px-4 pt-3 pb-3 border-b border-[hsl(var(--sidebar-border))]">
          <Link
            to="/teams"
            className="inline-flex items-center gap-1 text-xs text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--sidebar-foreground))] transition-colors mb-2.5"
          >
            <ArrowLeft className="w-3 h-3" />
            {t('sidebar.backToTeams')}
          </Link>
          <div className="flex items-center gap-2.5">
            <div className="w-9 h-9 rounded-lg bg-[hsl(var(--primary))] flex items-center justify-center shrink-0">
              <span className="text-white font-bold text-sm">
                {team.name.charAt(0).toUpperCase()}
              </span>
            </div>
            <div className="min-w-0">
              <h2 className="font-semibold text-sm leading-tight truncate">{team.name}</h2>
              {team.description && (
                <p className="text-caption text-[hsl(var(--muted-foreground))] leading-tight mt-0.5 truncate">
                  {team.description}
                </p>
              )}
            </div>
          </div>
          {canManage && (
            <Button
              size="sm"
              variant="outline"
              className="w-full mt-2.5 h-8 text-xs"
              onClick={onInviteClick}
            >
              <UserPlus className="h-3.5 w-3.5 mr-1.5" />
              {t('sidebar.inviteMembers')}
            </Button>
          )}
        </div>

        {/* Flat navigation */}
        <nav className="flex-1 overflow-y-auto px-3 py-3">
          <div className="space-y-px">
            {NAV_ITEMS.filter((item) => !item.adminOnly || canManage).map((item) => {
              const isActive = activeSection === item.key;
              const count = getNavCount(item.key, team);
              const icon = NAV_ICONS[item.icon];

              return (
                <div key={item.key}>
                  <button
                    type="button"
                    onClick={() => { onSectionChange(item.key); onNavigate?.(); }}
                    className={`w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-[13px] transition-colors ${
                      isActive
                        ? 'bg-[hsl(var(--sidebar-accent))] text-[hsl(var(--sidebar-accent-foreground))] font-medium'
                        : 'text-[hsl(var(--sidebar-foreground))] hover:bg-[hsl(var(--sidebar-accent))/0.5]'
                    }`}
                  >
                    <span className={isActive ? 'opacity-100' : 'opacity-50'}>{icon}</span>
                    <span className="flex-1 text-left">{t(item.labelKey)}</span>
                    {count !== null && (
                      <span className={`text-caption min-w-[1.25rem] text-center rounded-full px-1.5 py-px ${
                        isActive
                          ? 'bg-[hsl(var(--sidebar-accent-foreground))/0.15] text-[hsl(var(--sidebar-accent-foreground))]'
                          : 'bg-[hsl(var(--muted))] text-[hsl(var(--muted-foreground))]'
                      }`}>
                        {count}
                      </span>
                    )}
                  </button>
                  {SEPARATOR_AFTER.has(item.key) && (
                    <div className="my-2 border-t border-[hsl(var(--sidebar-border))]" />
                  )}
                </div>
              );
            })}
          </div>
        </nav>
      </>
    );
  };

  const renderDefaultNav = () => (
    <>
      {/* Logo */}
      <div className="p-4 border-b border-[hsl(var(--sidebar-border))]">
        <Link to="/dashboard" className="flex items-center gap-2">
          {brand.logoUrl ? (
            <img src={brand.logoUrl} alt={brand.name} className="w-8 h-8 rounded-lg object-contain" />
          ) : !brand.licensed ? (
            <img src={agimeLogoSvg} alt={brand.name} className="w-8 h-8 rounded-lg" />
          ) : (
            <div className="w-8 h-8 rounded-lg bg-[hsl(var(--primary))] flex items-center justify-center">
              <span className="text-white font-bold text-sm">{brand.logoText}</span>
            </div>
          )}
          <span className="font-semibold text-lg">{brand.name}</span>
        </Link>
      </div>

      {/* Navigation */}
      <nav className="flex-1 p-4 space-y-1">
        {defaultNavItems.filter((item) => !item.adminOnly || isAdmin).map((item) => {
          const isActive = location.pathname === item.path ||
            (item.path !== '/dashboard' && location.pathname.startsWith(item.path));

          return (
            <Link
              key={item.path}
              to={item.path}
              onClick={onNavigate}
              className={`flex items-center gap-3 px-3 py-2 rounded-lg transition-colors ${
                isActive
                  ? 'bg-[hsl(var(--sidebar-accent))] text-[hsl(var(--sidebar-accent-foreground))]'
                  : 'text-[hsl(var(--sidebar-foreground))] hover:bg-[hsl(var(--sidebar-accent))] hover:text-[hsl(var(--sidebar-accent-foreground))]'
              }`}
            >
              {item.icon}
              <span>{t(item.labelKey)}</span>
            </Link>
          );
        })}
      </nav>
    </>
  );

  // ── Collapsed user section ──
  const renderCollapsedUserSection = () => (
    <div className="py-3 border-t border-[hsl(var(--sidebar-border))] flex flex-col items-center gap-2">
      {teamCtx && (
        <button
          onClick={teamCtx.onToggleSidebar}
          title={t('sidebar.expand')}
          className="w-9 h-9 flex items-center justify-center rounded-md text-[hsl(var(--sidebar-foreground))] hover:bg-[hsl(var(--sidebar-accent))/0.5] transition-colors opacity-50 hover:opacity-100"
        >
          <PanelLeftOpen className="w-4 h-4" />
        </button>
      )}
      <ThemeToggle />
      <div
        className="w-8 h-8 rounded-full bg-[hsl(var(--muted))] flex items-center justify-center cursor-default"
        title={user?.display_name || ''}
      >
        <span className="text-sm font-medium">
          {user?.display_name?.charAt(0).toUpperCase() || 'U'}
        </span>
      </div>
      <div className="flex flex-col items-center gap-1.5 pt-1">
        {brand.websiteUrl && (
          <a href={brand.websiteUrl} target="_blank" rel="noopener noreferrer"
            className="text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--sidebar-foreground))] transition-colors"
            title={brand.websiteLabel || brand.name}>
            <Globe className="w-3.5 h-3.5" />
          </a>
        )}
        <a href="https://github.com/jsjm1986/AGIME" target="_blank" rel="noopener noreferrer"
          className="text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--sidebar-foreground))] transition-colors"
          title="GitHub">
          <Github className="w-3.5 h-3.5" />
        </a>
      </div>
    </div>
  );

  // ── Full user section (Option C: compact user row + branded footer) ──
  const renderUserSection = () => (
    <div className="border-t border-[hsl(var(--sidebar-border))]">
      {/* Tools row */}
      <div className="flex items-center justify-between px-4 pt-3 pb-1">
        <div className="flex items-center gap-1">
          <ThemeToggle />
          <LanguageSwitcher />
        </div>
        {teamCtx && (
          <button
            onClick={teamCtx.onToggleSidebar}
            title={t('sidebar.collapse')}
            className="w-8 h-8 flex items-center justify-center rounded-md text-[hsl(var(--sidebar-foreground))] hover:bg-[hsl(var(--sidebar-accent))/0.5] transition-colors opacity-50 hover:opacity-100"
          >
            <PanelLeftClose className="w-4 h-4" />
          </button>
        )}
      </div>

      {/* Compact user row */}
      <div className="flex items-center gap-2.5 px-4 py-2">
        <div className="w-7 h-7 rounded-full bg-[hsl(var(--muted))] flex items-center justify-center shrink-0">
          <span className="text-xs font-medium">
            {user?.display_name?.charAt(0).toUpperCase() || 'U'}
          </span>
        </div>
        <div className="flex-1 min-w-0">
          <p className="text-[13px] font-medium truncate leading-tight">{user?.display_name}</p>
          <p className="text-caption text-[hsl(var(--muted-foreground))] truncate leading-tight">{user?.email}</p>
        </div>
        <Button variant="ghost" size="sm" className="h-7 px-2 text-xs shrink-0" onClick={handleLogout}>
          {t('auth.logout')}
        </Button>
      </div>

      {/* Brand footer */}
      <div className="px-4 pt-2 pb-3 border-t border-[hsl(var(--sidebar-border))]">
        <div className="flex items-center justify-center gap-3 mb-1.5">
          {brand.websiteUrl && (
            <a href={brand.websiteUrl} target="_blank" rel="noopener noreferrer"
              className="flex items-center gap-1 text-caption text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--sidebar-foreground))] transition-colors">
              <Globe className="w-3 h-3" />
              <span>{brand.websiteLabel || t('sidebar.website')}</span>
            </a>
          )}
          <a href="https://github.com/jsjm1986/AGIME" target="_blank" rel="noopener noreferrer"
            className="flex items-center gap-1 text-caption text-[hsl(var(--muted-foreground))] hover:text-[hsl(var(--sidebar-foreground))] transition-colors">
            <Github className="w-3 h-3" />
            <span>GitHub</span>
          </a>
        </div>
        {brand.poweredByVisible && (
          <p className="text-micro text-center text-gray-400">
            Powered by Agime Team
          </p>
        )}
      </div>
    </div>
  );

  return (
    <aside
      className={`h-full flex flex-col bg-[hsl(var(--sidebar-background))] border-r border-[hsl(var(--sidebar-border))] transition-[width] duration-200 shrink-0 ${
        collapsed ? 'w-14' : 'w-64'
      }`}
    >
      {teamCtx
        ? (collapsed ? renderCollapsedTeamNav() : renderTeamNav())
        : renderDefaultNav()
      }
      {collapsed ? renderCollapsedUserSection() : renderUserSection()}
    </aside>
  );
}
