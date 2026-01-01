import { createContext, useContext, useState, useEffect, useCallback, useMemo, ReactNode } from 'react';

const STORAGE_KEY = 'agime-pending-extension-link';

interface ExtensionInstallContextType {
  pendingLink: string | null;
  setPendingLink: (link: string | null) => void;
  clearPendingLink: () => void;
  error: string | null;
  setError: (error: string | null) => void;
  isPending: boolean;
  setIsPending: (isPending: boolean) => void;
}

const ExtensionInstallContext = createContext<ExtensionInstallContextType | undefined>(undefined);

interface ExtensionInstallProviderProps {
  children: ReactNode;
}

export function ExtensionInstallProvider({ children }: ExtensionInstallProviderProps) {
  // Initialize from localStorage to recover pending installations after app restart
  const [pendingLink, setPendingLinkState] = useState<string | null>(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      return stored || null;
    } catch {
      return null;
    }
  });

  const [error, setErrorState] = useState<string | null>(null);
  const [isPending, setIsPendingState] = useState<boolean>(false);

  // Persist to localStorage when pendingLink changes (single source of truth)
  useEffect(() => {
    try {
      if (pendingLink) {
        localStorage.setItem(STORAGE_KEY, pendingLink);
      } else {
        localStorage.removeItem(STORAGE_KEY);
      }
    } catch (err) {
      console.error('Failed to save pending extension link:', err);
    }
  }, [pendingLink]);

  // Wrap all setters with useCallback for stable references
  const setPendingLink = useCallback((link: string | null) => {
    setPendingLinkState(link);
    // Clear error when setting a new link
    if (link) {
      setErrorState(null);
    }
  }, []);

  const clearPendingLink = useCallback(() => {
    setPendingLinkState(null);
    setErrorState(null);
    setIsPendingState(false);
    // Note: localStorage cleanup handled by useEffect above
  }, []);

  const setError = useCallback((error: string | null) => {
    setErrorState(error);
  }, []);

  const setIsPending = useCallback((pending: boolean) => {
    setIsPendingState(pending);
  }, []);

  // Memoize context value to prevent unnecessary re-renders of consumers
  const contextValue = useMemo(
    () => ({
      pendingLink,
      setPendingLink,
      clearPendingLink,
      error,
      setError,
      isPending,
      setIsPending,
    }),
    [pendingLink, setPendingLink, clearPendingLink, error, setError, isPending, setIsPending]
  );

  return (
    <ExtensionInstallContext.Provider value={contextValue}>
      {children}
    </ExtensionInstallContext.Provider>
  );
}

export function useExtensionInstall(): ExtensionInstallContextType {
  const context = useContext(ExtensionInstallContext);
  if (context === undefined) {
    throw new Error('useExtensionInstall must be used within an ExtensionInstallProvider');
  }
  return context;
}

export { ExtensionInstallContext };
