import { createContext, useContext, ReactNode } from 'react';
import type { TeamWithStats } from '../api/types';

interface TeamContextType {
  team: TeamWithStats;
  canManage: boolean;
  activeSection: string;
  onSectionChange: (section: string) => void;
  onInviteClick: () => void;
  sidebarCollapsed: boolean;
  onToggleSidebar: () => void;
}

const TeamContext = createContext<TeamContextType | null>(null);

interface TeamProviderProps {
  value: TeamContextType;
  children: ReactNode;
}

export function TeamProvider({ value, children }: TeamProviderProps) {
  return (
    <TeamContext.Provider value={value}>
      {children}
    </TeamContext.Provider>
  );
}

export function useTeamContext(): TeamContextType | null {
  return useContext(TeamContext);
}
