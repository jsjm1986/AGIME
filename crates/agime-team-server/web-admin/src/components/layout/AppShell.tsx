import { ReactNode, useMemo, useState } from 'react';
import { FileText, Menu, MessageSquareText, Package, Workflow } from 'lucide-react';
import { useLocation, useNavigate, useSearchParams } from 'react-router-dom';
import { Sidebar } from './Sidebar';
import { useIsMobile, useMediaQuery } from '../../hooks/useMediaQuery';
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
  const isMobileWorkspace = useMediaQuery('(max-width: 1023px)');
  const location = useLocation();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const { brand } = useBrand();
  const teamCtx = useTeamContext();
  const { isConversationMode } = useMobileInteractionMode();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);
  const isTeamWorkspaceRoute =
    location.pathname.startsWith('/teams/') &&
    !location.pathname.includes('/system-admin');
  const showModeSwitch = isMobileWorkspace && isTeamWorkspaceRoute;
  const activeAgentTab = searchParams.get('agentTab');
  const conversationWorkspaceActiveObject = useMemo(() => {
    const activeSection = teamCtx?.activeSection;
    if (!activeSection) return null;
    if (activeSection === 'documents') return 'documents';
    if (activeSection === 'toolkit') return 'resources';
    if (
      (activeSection === 'agent' && (activeAgentTab === 'missions' || activeAgentTab === 'task-queue')) ||
      activeSection === 'smart-log'
    ) {
      return 'tasks';
    }
    if (activeSection === 'chat' || activeSection === 'agent' || activeSection === 'digital-avatar') {
      return 'dialogue';
    }
    return null;
  }, [activeAgentTab, teamCtx?.activeSection]);
  const showConversationObjectNav =
    Boolean(teamCtx) &&
    isTeamWorkspaceRoute &&
    isMobileWorkspace &&
    isConversationMode &&
    Boolean(conversationWorkspaceActiveObject);

  const handleConversationObjectChange = (nextObject: 'dialogue' | 'tasks' | 'documents' | 'resources') => {
    if (!teamCtx) return;
    const nextParams = new URLSearchParams();
    switch (nextObject) {
      case 'dialogue':
        nextParams.set('section', teamCtx.activeSection === 'digital-avatar' ? 'digital-avatar' : 'chat');
        break;
      case 'tasks':
        nextParams.set('section', 'agent');
        nextParams.set('agentTab', 'missions');
        break;
      case 'documents':
        nextParams.set('section', 'documents');
        break;
      case 'resources':
        nextParams.set('section', 'toolkit');
        break;
    }
    navigate(`/teams/${teamCtx.team.id}?${nextParams.toString()}`);
  };

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
      <main className="relative min-w-0 flex-1 overflow-auto">
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
            {showConversationObjectNav ? (
              <div className="mt-2 space-y-2">
                <div className="grid grid-cols-4 gap-2">
                  {[
                    { key: 'dialogue', label: '对话', icon: MessageSquareText },
                    { key: 'tasks', label: '任务', icon: Workflow },
                    { key: 'documents', label: '文档', icon: FileText },
                    { key: 'resources', label: '资源', icon: Package },
                  ].map((item) => {
                    const Icon = item.icon;
                    const isActive = conversationWorkspaceActiveObject === item.key;
                    return (
                      <button
                        key={item.key}
                        type="button"
                        onClick={() => handleConversationObjectChange(item.key as 'dialogue' | 'tasks' | 'documents' | 'resources')}
                        className={`flex h-10 items-center justify-center gap-1 rounded-[16px] border px-2 text-[12px] font-medium transition-colors ${
                          isActive
                            ? 'border-primary/35 bg-primary/10 text-primary'
                            : 'border-border/70 bg-card/75 text-muted-foreground'
                        }`}
                      >
                        <Icon className="h-3.5 w-3.5" />
                        <span className="truncate">{item.label}</span>
                      </button>
                    );
                  })}
                </div>
                {teamCtx?.activeSection === 'digital-avatar' ? (
                  <div className="inline-flex items-center rounded-full border border-[hsl(var(--semantic-portal))]/25 bg-[hsl(var(--semantic-portal))]/10 px-3 py-1 text-[11px] font-semibold tracking-[0.02em] text-[hsl(var(--semantic-portal))]">
                    数字分身上下文
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>
        )}
        <div className={isMobile ? 'p-3' : 'p-5 lg:p-6'}>
          {children}
        </div>
      </main>
    </div>
  );
}
