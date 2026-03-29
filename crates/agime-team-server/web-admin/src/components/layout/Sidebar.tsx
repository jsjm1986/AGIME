import { Link, useLocation } from "react-router-dom";
import { useTranslation } from "react-i18next";
import { useAuth } from "../../contexts/AuthContext";
import { useTeamContext } from "../../contexts/TeamContext";
import { ThemeToggle } from "../ThemeToggle";
import { LanguageSwitcher } from "../LanguageSwitcher";
import { Button } from "../ui/button";
import {
  UserPlus,
  ArrowLeft,
  Zap,
  FileText,
  Bot,
  MessageCircle,
  Users,
  ScrollText,
  Handshake,
  FlaskConical,
  UserRound,
  PanelLeftClose,
  PanelLeftOpen,
  Globe,
  Github,
  ShieldCheck,
} from "lucide-react";
import { NAV_ITEMS } from "../../config/teamNavConfig";
import { useBrand } from "../../contexts/BrandContext";
import agimeLogoSvg from "../../assets/agime-logo.svg";
import { RelationshipMemoryControl } from "../chat/RelationshipMemoryControl";

const NAV_ICONS: Record<string, React.ReactNode> = {
  MessageCircle: <MessageCircle className="w-4 h-4" />,
  Bot: <Bot className="w-4 h-4" />,
  FileText: <FileText className="w-4 h-4" />,
  Zap: <Zap className="w-4 h-4" />,
  ScrollText: <ScrollText className="w-4 h-4" />,
  Handshake: <Handshake className="w-4 h-4" />,
  FlaskConical: <FlaskConical className="w-4 h-4" />,
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
    case "toolkit":
      return team.skillsCount ?? 0;
    case "team-admin":
      return team.membersCount ?? 0;
    default:
      return null;
  }
}

/** Keys after which a visual separator is rendered */
const SEPARATOR_AFTER = new Set(["chat", "smart-log", "external-users"]);

interface NavItem {
  path: string;
  labelKey: string;
  icon: React.ReactNode;
  adminOnly?: boolean;
}

const systemAdminNavItems: NavItem[] = [
  {
    path: "/system-admin",
    labelKey: "sidebar.systemAdmin",
    icon: <ShieldCheck className="w-5 h-5" />,
  },
];

const defaultNavItems: NavItem[] = [
  {
    path: "/dashboard",
    labelKey: "sidebar.dashboard",
    icon: (
      <svg
        className="w-5 h-5"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M3 12l2-2m0 0l7-7 7 7M5 10v10a1 1 0 001 1h3m10-11l2 2m-2-2v10a1 1 0 01-1 1h-3m-6 0a1 1 0 001-1v-4a1 1 0 011-1h2a1 1 0 011 1v4a1 1 0 001 1m-6 0h6"
        />
      </svg>
    ),
  },
  {
    path: "/teams",
    labelKey: "sidebar.teams",
    icon: (
      <svg
        className="w-5 h-5"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z"
        />
      </svg>
    ),
  },
  {
    path: "/api-keys",
    labelKey: "sidebar.apiKeys",
    icon: (
      <svg
        className="w-5 h-5"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"
        />
      </svg>
    ),
  },
  {
    path: "/settings",
    labelKey: "sidebar.settings",
    icon: (
      <svg
        className="w-5 h-5"
        fill="none"
        viewBox="0 0 24 24"
        stroke="currentColor"
      >
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
        />
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
        />
      </svg>
    ),
  },
  {
    path: "/system-admin",
    labelKey: "sidebar.systemAdmin",
    icon: <ShieldCheck className="w-5 h-5" />,
    adminOnly: true,
  },
];

interface SidebarProps {
  onNavigate?: () => void;
}

function truncateFooterLabel(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, Math.max(0, maxLength - 1))}…`;
}

interface SidebarHeaderBlockProps {
  utility?: React.ReactNode;
  children: React.ReactNode;
}

function SidebarHeaderBlock({
  utility,
  children,
}: SidebarHeaderBlockProps) {
  return (
    <div className="border-b border-[hsl(var(--sidebar-border))] px-3 pb-2.5 pt-3">
      {utility ? (
        <div className="mb-2 flex items-center justify-between gap-2">
          {utility}
        </div>
      ) : null}
      <div className="space-y-2">{children}</div>
    </div>
  );
}

interface SidebarFooterProps {
  onLogout: () => void;
  userName: string;
  logoutLabel: string;
  websiteTitle: string;
  websiteText: string;
  websiteUrl?: string | null;
  githubLabel: string;
  poweredByText: string;
  showPoweredBy: boolean;
  relationshipMemoryControl?: React.ReactNode;
}

function SidebarFooter({
  onLogout,
  userName,
  logoutLabel,
  websiteTitle,
  websiteText,
  websiteUrl,
  githubLabel,
  poweredByText,
  showPoweredBy,
  relationshipMemoryControl,
}: SidebarFooterProps) {
  const controlTextButtonClass =
    "inline-flex h-5 shrink-0 items-center justify-center whitespace-nowrap rounded-full border border-transparent bg-transparent px-1 text-[9px] font-normal leading-4 tracking-[0.01em] text-[hsl(var(--sidebar-foreground))/0.68] transition-colors hover:text-[hsl(var(--sidebar-foreground))]";
  const footerLinkClass =
    "inline-flex items-center gap-1 text-[9px] font-normal leading-4 text-[hsl(var(--sidebar-foreground))/0.62] transition-colors hover:text-[hsl(var(--sidebar-foreground))]";

  return (
    <div className="border-t border-[hsl(var(--sidebar-border))] px-3 pb-2 pt-2">
      <div className="space-y-1.5">
        <div className="flex justify-center">
          <div className="inline-flex items-center gap-1 text-[hsl(var(--sidebar-foreground))/0.72]">
            <ThemeToggle className="h-5 w-5 rounded-full border border-transparent bg-transparent p-0 text-[hsl(var(--sidebar-foreground))/0.68] shadow-none hover:bg-[hsl(var(--sidebar-accent))/0.05] hover:text-[hsl(var(--sidebar-foreground))]" />
            <LanguageSwitcher
              className={`${controlTextButtonClass} shadow-none`}
            />
            <span className="text-[10px] text-[hsl(var(--sidebar-foreground))/0.28]">
              ·
            </span>
            <button
              type="button"
              className={controlTextButtonClass}
              onClick={onLogout}
            >
              {logoutLabel}
            </button>
          </div>
        </div>

        <div className="flex justify-center">
          <div className="flex min-w-0 items-center justify-center gap-1.5">
            <p className="max-w-[144px] truncate text-center text-[11px] font-normal tracking-[0.01em] leading-4 text-[hsl(var(--sidebar-foreground))]">
              {userName}
            </p>
            {relationshipMemoryControl ? relationshipMemoryControl : null}
          </div>
        </div>

        <div className="flex flex-wrap items-center justify-center gap-x-2.5 gap-y-1 px-0.5 pt-0.5">
          {websiteUrl ? (
            <a
              href={websiteUrl}
              target="_blank"
              rel="noopener noreferrer"
              className={footerLinkClass}
              title={websiteTitle}
            >
              <Globe className="h-3 w-3 shrink-0" />
              <span>{websiteText}</span>
            </a>
          ) : null}
          <a
            href="https://github.com/jsjm1986/AGIME"
            target="_blank"
            rel="noopener noreferrer"
            className={footerLinkClass}
          >
            <Github className="h-3 w-3 shrink-0" />
            <span>{githubLabel}</span>
          </a>
        </div>
        {showPoweredBy ? (
          <p className="px-0.5 pt-0.5 text-center text-[9px] font-normal leading-4 text-[hsl(var(--sidebar-foreground))/0.4]">
            {poweredByText}
          </p>
        ) : null}
      </div>
    </div>
  );
}

export function Sidebar({ onNavigate }: SidebarProps = {}) {
  const { t } = useTranslation();
  const { brand } = useBrand();
  const { user, logout, isAdmin, authMode } = useAuth();
  const location = useLocation();
  const teamCtx = useTeamContext();
  const isSystemAdminSession = authMode === "system-admin";
  const homePath = isSystemAdminSession ? "/system-admin" : "/dashboard";
  const navItems = isSystemAdminSession
    ? systemAdminNavItems
    : defaultNavItems.filter((item) => !item.adminOnly || isAdmin);

  const collapsed = teamCtx?.sidebarCollapsed ?? false;

  const handleLogout = async () => {
    await logout();
  };

  const navBodyClass = "flex-1 overflow-y-auto px-3 py-3";
  const navListClass = "space-y-1";
  const headerUtilityLinkClass =
    "inline-flex items-center gap-1 text-[11px] font-medium text-[hsl(var(--sidebar-foreground))/0.68] transition-colors hover:text-[hsl(var(--sidebar-foreground))]";
  const headerUtilityButtonClass =
    "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-[12px] border border-[hsl(var(--sidebar-border))/0.82] bg-[hsl(var(--sidebar-surface))] text-[hsl(var(--sidebar-foreground))/0.74] transition-colors hover:border-[hsl(var(--sidebar-accent))/0.22] hover:bg-[hsl(var(--sidebar-accent))/0.08] hover:text-[hsl(var(--sidebar-foreground))]";
  const getNavItemClass = (isActive: boolean) =>
    `w-full flex items-center gap-2 rounded-[12px] px-2.5 py-2 text-[13px] transition-colors ${
      isActive
        ? "bg-[hsl(var(--sidebar-accent))/0.11] text-[hsl(var(--sidebar-foreground))] font-medium"
        : "text-[hsl(var(--sidebar-foreground))/0.86] hover:bg-[hsl(var(--sidebar-accent))/0.06] hover:text-[hsl(var(--sidebar-foreground))]"
    }`;

  const renderBrandMark = () => (
    <>
      {brand.logoUrl ? (
        <img
          src={brand.logoUrl}
          alt={brand.name}
          className="h-10 w-10 rounded-[14px] object-contain"
        />
      ) : !brand.licensed ? (
        <img
          src={agimeLogoSvg}
          alt={brand.name}
          className="h-10 w-10 rounded-[14px]"
        />
      ) : (
        <div className="flex h-10 w-10 items-center justify-center rounded-[14px] bg-[hsl(var(--primary))/0.12]">
          <span className="text-sm font-bold text-[hsl(var(--sidebar-foreground))]">
            {brand.logoText}
          </span>
        </div>
      )}
    </>
  );

  // ── Collapsed team nav (icon rail) ──
  const renderCollapsedTeamNav = () => {
    if (!teamCtx) return null;
    const { team, activeSection, onSectionChange } = teamCtx;

    return (
      <>
        <div className="border-b border-[hsl(var(--sidebar-border))] px-2 pb-2.5 pt-3">
          <div className="flex justify-center">
            <div className="flex h-9 w-9 items-center justify-center rounded-[14px] border border-[hsl(var(--sidebar-border))/0.82] bg-[hsl(var(--sidebar-surface))] text-sm font-semibold text-[hsl(var(--sidebar-foreground))] shadow-[0_1px_0_hsl(var(--sidebar-border))/0.12]">
              {team.name.charAt(0).toUpperCase()}
            </div>
          </div>
        </div>

        <nav className="flex-1 overflow-y-auto px-2 py-3">
          <div className="flex flex-col items-center gap-1">
            {NAV_ITEMS.filter((item) => !item.adminOnly || teamCtx.canManage).map(
              (item) => {
                const isActive = activeSection === item.key;
                const icon = NAV_ICONS[item.icon];
                return (
                  <div key={item.key}>
                    <button
                      type="button"
                      onClick={() => {
                        onSectionChange(item.key);
                        onNavigate?.();
                      }}
                      title={t(item.labelKey)}
                      className={`flex h-9 w-9 items-center justify-center rounded-[12px] border transition-colors ${
                        isActive
                          ? "border-[hsl(var(--sidebar-accent))/0.22] bg-[hsl(var(--sidebar-accent))/0.12] text-[hsl(var(--sidebar-foreground))]"
                          : "border-transparent bg-transparent text-[hsl(var(--sidebar-foreground))/0.72] hover:border-[hsl(var(--sidebar-border))/0.82] hover:bg-[hsl(var(--sidebar-surface))] hover:text-[hsl(var(--sidebar-foreground))]"
                      }`}
                    >
                      <span className={isActive ? "opacity-100" : "opacity-82"}>
                        {icon}
                      </span>
                    </button>
                    {SEPARATOR_AFTER.has(item.key) && (
                      <div className="my-2 mx-auto w-6 border-t border-[hsl(var(--sidebar-border))]" />
                    )}
                  </div>
                );
              },
            )}
          </div>
        </nav>
      </>
    );
  };

  // ── Full team nav ──
  const renderTeamNav = () => {
    if (!teamCtx) return null;
    const { team, canManage, activeSection, onSectionChange, onInviteClick } =
      teamCtx;

    return (
      <>
        <SidebarHeaderBlock
          utility={
            <>
              <Link to="/teams" className={headerUtilityLinkClass}>
                <ArrowLeft className="h-3 w-3" />
                {t("sidebar.backToTeams")}
              </Link>
              <button
                type="button"
                onClick={teamCtx.onToggleSidebar}
                title={t("sidebar.collapse")}
                className={headerUtilityButtonClass}
              >
                <PanelLeftClose className="h-3.5 w-3.5" strokeWidth={1.85} />
              </button>
            </>
          }
        >
            <div className="flex items-center gap-2.5">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[14px] border border-[hsl(var(--sidebar-border))/0.82] bg-[hsl(var(--sidebar-surface))]">
                <span className="text-sm font-semibold text-[hsl(var(--sidebar-foreground))]">
                  {team.name.charAt(0).toUpperCase()}
                </span>
              </div>
              <div className="min-w-0">
                <h2 className="truncate text-sm font-semibold leading-tight text-[hsl(var(--sidebar-foreground))]">
                  {team.name}
                </h2>
                {team.description && (
                  <p className="mt-0.5 truncate text-[11px] leading-tight text-[hsl(var(--sidebar-foreground))/0.7]">
                    {team.description}
                  </p>
                )}
              </div>
            </div>
            {canManage && (
              <Button
                size="sm"
                variant="outline"
                className="mt-2 h-8 w-full rounded-[12px] border-[hsl(var(--sidebar-border))/0.82] bg-[hsl(var(--sidebar-surface))] text-[11px] text-[hsl(var(--sidebar-foreground))] hover:border-[hsl(var(--sidebar-accent))/0.24] hover:bg-[hsl(var(--sidebar-accent))/0.08] hover:text-[hsl(var(--sidebar-foreground))]"
                onClick={onInviteClick}
              >
                <UserPlus className="mr-1.5 h-3.5 w-3.5" />
                {t("sidebar.inviteMembers")}
              </Button>
            )}
        </SidebarHeaderBlock>

        <nav className={navBodyClass}>
          <div className={navListClass}>
            {NAV_ITEMS.filter((item) => !item.adminOnly || canManage).map(
              (item) => {
                const isActive = activeSection === item.key;
                const count = getNavCount(item.key, team);
                const icon = NAV_ICONS[item.icon];

                return (
                  <div key={item.key}>
                    <button
                      type="button"
                      onClick={() => {
                        onSectionChange(item.key);
                        onNavigate?.();
                      }}
                      className={getNavItemClass(isActive)}
                    >
                      <span className={isActive ? "opacity-100" : "opacity-78"}>
                        {icon}
                      </span>
                      <span className="flex-1 text-left">
                        {t(item.labelKey)}
                      </span>
                      {count !== null && (
                        <span
                          className={`text-caption min-w-[1.25rem] text-center rounded-full px-1.5 py-px ${
                            isActive
                              ? "bg-[hsl(var(--sidebar-accent-foreground))/0.14] text-[hsl(var(--sidebar-accent-foreground))]"
                              : "bg-[hsl(var(--muted))] text-[hsl(var(--muted-foreground))]"
                          }`}
                        >
                          {count}
                        </span>
                      )}
                    </button>
                    {SEPARATOR_AFTER.has(item.key) && (
                      <div className="my-2 border-t border-[hsl(var(--sidebar-border))]" />
                    )}
                  </div>
                );
              },
            )}
          </div>
        </nav>
      </>
    );
  };

  const renderDefaultNav = () => (
    <>
      <SidebarHeaderBlock>
          <Link to={homePath} className="flex items-center gap-2.5">
            <div className="shrink-0">{renderBrandMark()}</div>
            <div className="min-w-0">
              <h2 className="truncate text-sm font-semibold leading-tight text-[hsl(var(--sidebar-foreground))]">
                {brand.name}
              </h2>
              <p className="mt-0.5 truncate text-[11px] leading-tight text-[hsl(var(--sidebar-foreground))/0.7]">
                {isSystemAdminSession
                  ? t("sidebar.systemAdmin")
                  : t("sidebar.teams")}
              </p>
            </div>
          </Link>
      </SidebarHeaderBlock>

      <nav className={navBodyClass}>
        <div className={navListClass}>
          {navItems.map((item, index) => {
          const isActive =
            location.pathname === item.path ||
            (item.path !== "/dashboard" &&
              location.pathname.startsWith(item.path));
          const shouldRenderDivider =
            index > 0 &&
            ((isSystemAdminSession && item.path === "/system-admin") ||
              (!isSystemAdminSession && item.path === "/system-admin"));

          return (
            <div key={item.path}>
              {shouldRenderDivider ? (
                <div className="my-2 border-t border-[hsl(var(--sidebar-border))]" />
              ) : null}
              <Link
                to={item.path}
                onClick={onNavigate}
                className={getNavItemClass(isActive)}
              >
                <span className={isActive ? "opacity-100" : "opacity-78"}>
                  {item.icon}
                </span>
                <span className="flex-1 text-left">{t(item.labelKey)}</span>
              </Link>
            </div>
          );
        })}
        </div>
      </nav>
    </>
  );

  // ── Collapsed user section ──
  const renderCollapsedUserSection = () => (
    <div className="border-t border-[hsl(var(--sidebar-border))] px-2 pb-2.5 pt-2.5">
      <div className="flex flex-col items-center gap-2">
        {teamCtx ? (
          <button
            onClick={teamCtx.onToggleSidebar}
            title={t("sidebar.expand")}
            className="flex h-8 w-8 items-center justify-center rounded-[12px] border border-[hsl(var(--sidebar-border))/0.82] bg-[hsl(var(--sidebar-surface))] text-[hsl(var(--sidebar-foreground))/0.82] transition-colors hover:border-[hsl(var(--sidebar-accent))/0.22] hover:bg-[hsl(var(--sidebar-accent))/0.08] hover:text-[hsl(var(--sidebar-foreground))]"
          >
            <PanelLeftOpen className="h-3.5 w-3.5" />
          </button>
          ) : null}
        {teamCtx && user ? (
          <RelationshipMemoryControl
            teamId={teamCtx.team.id}
            teamName={teamCtx.team.name}
            userDisplayName={user.display_name}
            variant="icon"
          />
        ) : null}
        <ThemeToggle className="h-8 w-8 rounded-[12px] border border-[hsl(var(--sidebar-border))/0.82] bg-[hsl(var(--sidebar-surface))] p-0 text-[hsl(var(--sidebar-foreground))/0.82] hover:border-[hsl(var(--sidebar-accent))/0.22] hover:bg-[hsl(var(--sidebar-accent))/0.08] hover:text-[hsl(var(--sidebar-foreground))]" />
        <div className="flex flex-col items-center gap-1">
          {brand.websiteUrl ? (
            <a
              href={brand.websiteUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-[hsl(var(--sidebar-foreground))/0.68] transition-colors hover:text-[hsl(var(--sidebar-foreground))]"
              title={brand.websiteLabel || brand.name}
            >
              <Globe className="h-3.5 w-3.5" />
            </a>
          ) : null}
          <a
            href="https://github.com/jsjm1986/AGIME"
            target="_blank"
            rel="noopener noreferrer"
            className="text-[hsl(var(--sidebar-foreground))/0.68] transition-colors hover:text-[hsl(var(--sidebar-foreground))]"
            title="GitHub"
          >
            <Github className="h-3.5 w-3.5" />
          </a>
        </div>
      </div>
    </div>
  );
  const footerWebsiteTitle = brand.websiteLabel || brand.name;
  const footerWebsiteText = t("sidebar.website");
  const footerUserName = truncateFooterLabel(user?.display_name || brand.name, 16);

  return (
    <aside
      className={`h-full shrink-0 border-r border-[hsl(var(--sidebar-border))] bg-[hsl(var(--sidebar-background))] text-[hsl(var(--sidebar-foreground))] shadow-[4px_0_14px_hsl(var(--ui-shadow))/0.03] transition-[width] duration-200 flex flex-col dark:shadow-[8px_0_18px_hsl(var(--ui-shadow))/0.18] ${
        collapsed ? "w-14" : "w-64"
      }`}
    >
      {teamCtx
        ? collapsed
          ? renderCollapsedTeamNav()
          : renderTeamNav()
        : renderDefaultNav()}
      {collapsed ? (
        renderCollapsedUserSection()
      ) : (
        <SidebarFooter
          onLogout={handleLogout}
          userName={footerUserName}
          logoutLabel={t("auth.logout")}
          websiteTitle={footerWebsiteTitle}
          websiteText={footerWebsiteText}
          websiteUrl={brand.websiteUrl}
          githubLabel={t("sidebar.github")}
          poweredByText={t("sidebar.poweredBy")}
          showPoweredBy={brand.poweredByVisible}
          relationshipMemoryControl={
            teamCtx && user ? (
              <RelationshipMemoryControl
                teamId={teamCtx.team.id}
                teamName={teamCtx.team.name}
                userDisplayName={user.display_name}
              />
            ) : undefined
          }
        />
      )}
    </aside>
  );
}
