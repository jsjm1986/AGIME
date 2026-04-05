import { ReactNode, useState } from 'react';
import { Menu } from 'lucide-react';
import { useLocation } from 'react-router-dom';
import { Sidebar } from './Sidebar';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { useBrand } from '../../contexts/BrandContext';
import { MobileInteractionModeSwitch } from '../mobile/MobileInteractionModeSwitch';
import { useTeamContext } from '../../contexts/TeamContext';
import { useMobileInteractionMode } from '../../contexts/MobileInteractionModeContext';

interface AppShellProps {
  children: ReactNode;
  className?: string;
}

export function AppShell({ children, className = '' }: AppShellProps) {
  const isMobile = useIsMobile();
  const location = useLocation();
  const { brand } = useBrand();
  const teamCtx = useTeamContext();
  const { isMobileWorkspace: isConversationWorkspace } = useMobileInteractionMode();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const isTeamWorkspaceRoute =
    location.pathname.startsWith('/teams/') &&
    !location.pathname.includes('/system-admin');
  const showModeSwitch = isConversationWorkspace && isTeamWorkspaceRoute;
  const isCollaborationWorkspace =
    Boolean(teamCtx) && isTeamWorkspaceRoute && teamCtx?.activeSection === 'collaboration';

  return (
    <div className={`h-screen flex bg-[hsl(var(--background))] ${className}`}>
      {isMobile ? (
        <>
          {mobileMenuOpen && (
            <div className="fixed inset-0 z-40 flex">
              <div className="fixed inset-0 bg-black/40" onClick={() => setMobileMenuOpen(false)} />
              <div className="relative z-50">
                <Sidebar onNavigate={() => setMobileMenuOpen(false)} />
              </div>
            </div>
          )}
        </>
      ) : (
        <Sidebar />
      )}
      <main className={`relative min-w-0 flex-1 ${isCollaborationWorkspace ? 'min-h-0 overflow-hidden' : 'overflow-auto'}`}>
        {(isMobile || showModeSwitch) && (
          <div className="sticky top-0 z-30 border-b border-border/80 bg-[hsl(var(--background))/0.94] px-3 py-2.5 backdrop-blur-sm">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex min-w-0 items-center gap-2">
                {isMobile ? (
                  <button onClick={() => setMobileMenuOpen(true)} className="rounded-[calc(var(--radius)+4px)] border border-border/70 bg-card/85 p-2 shadow-sm transition-colors hover:bg-accent/70">
                    <Menu className="w-5 h-5" />
                  </button>
                ) : null}
                <span className="truncate text-[13px] font-semibold tracking-[0.01em]">{brand.name}</span>
              </div>
              {showModeSwitch ? <MobileInteractionModeSwitch /> : null}
            </div>
            {teamCtx?.activeSection === 'digital-avatar' ? (
              <div className="mt-2">
                <div className="inline-flex items-center rounded-full border border-[hsl(var(--semantic-portal))]/25 bg-[hsl(var(--semantic-portal))]/10 px-3 py-1 text-[11px] font-semibold tracking-[0.02em] text-[hsl(var(--semantic-portal))]">
                  数字分身上下文
                </div>
              </div>
            ) : null}
          </div>
        )}
        <div
          className={
            isCollaborationWorkspace
              ? 'h-full min-h-0'
              : isMobile
                ? 'p-3'
                : 'p-5 lg:p-6'
          }
        >
          {children}
        </div>
      </main>
    </div>
  );
}
