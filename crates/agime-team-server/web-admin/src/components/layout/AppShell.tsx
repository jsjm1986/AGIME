import { ReactNode, useState } from 'react';
import { Menu } from 'lucide-react';
import { Sidebar } from './Sidebar';
import { useIsMobile } from '../../hooks/useMediaQuery';
import { useBrand } from '../../contexts/BrandContext';

interface AppShellProps {
  children: ReactNode;
  className?: string;
}

export function AppShell({ children, className = '' }: AppShellProps) {
  const isMobile = useIsMobile();
  const { brand } = useBrand();
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false);

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
        {isMobile && (
          <div className="sticky top-0 z-30 flex items-center gap-2 border-b border-border/80 bg-[hsl(var(--background))/0.9] px-3 py-2.5 backdrop-blur-sm">
            <button onClick={() => setMobileMenuOpen(true)} className="rounded-[calc(var(--radius)+4px)] border border-border/70 bg-card/85 p-2 shadow-sm transition-colors hover:bg-accent/70">
              <Menu className="w-5 h-5" />
            </button>
            <span className="text-[13px] font-semibold tracking-[0.01em]">{brand.name}</span>
          </div>
        )}
        <div className={isMobile ? 'p-3' : 'p-5 lg:p-6'}>
          {children}
        </div>
      </main>
    </div>
  );
}
