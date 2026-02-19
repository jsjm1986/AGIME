import { ReactNode, useState } from 'react';
import { Menu } from 'lucide-react';
import { Sidebar } from './Sidebar';
import { useIsMobile } from '../../hooks/useMediaQuery';

interface AppShellProps {
  children: ReactNode;
}

export function AppShell({ children }: AppShellProps) {
  const isMobile = useIsMobile();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);

  return (
    <div className="min-h-screen flex bg-[hsl(var(--background))]">
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
      <main className="flex-1 overflow-auto min-w-0">
        {isMobile && (
          <div className="sticky top-0 z-30 flex items-center gap-2 px-3 py-2 bg-[hsl(var(--background))] border-b">
            <button onClick={() => setMobileMenuOpen(true)} className="p-1.5 rounded-md hover:bg-muted">
              <Menu className="w-5 h-5" />
            </button>
            <span className="text-sm font-medium">Agime Team</span>
          </div>
        )}
        <div className={isMobile ? 'p-3' : 'p-5'}>
          {children}
        </div>
      </main>
    </div>
  );
}
