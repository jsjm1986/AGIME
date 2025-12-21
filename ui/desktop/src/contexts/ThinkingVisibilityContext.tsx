import { createContext, useContext, useState, useEffect, useCallback, ReactNode } from 'react';

const STORAGE_KEY = 'agime-show-thinking';

interface ThinkingVisibilityContextType {
  showThinking: boolean;
  toggleThinking: () => void;
  setShowThinking: (value: boolean) => void;
}

const ThinkingVisibilityContext = createContext<ThinkingVisibilityContextType | undefined>(undefined);

interface ThinkingVisibilityProviderProps {
  children: ReactNode;
}

export function ThinkingVisibilityProvider({ children }: ThinkingVisibilityProviderProps) {
  // Initialize from localStorage, default to true (show thinking)
  const [showThinking, setShowThinkingState] = useState<boolean>(() => {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      return stored === null ? true : stored === 'true';
    } catch {
      return true;
    }
  });

  // Persist to localStorage when value changes
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, String(showThinking));
    } catch (error) {
      console.error('Failed to save thinking visibility preference:', error);
    }
  }, [showThinking]);

  const toggleThinking = useCallback(() => {
    setShowThinkingState((prev) => !prev);
  }, []);

  const setShowThinking = useCallback((value: boolean) => {
    setShowThinkingState(value);
  }, []);

  return (
    <ThinkingVisibilityContext.Provider value={{ showThinking, toggleThinking, setShowThinking }}>
      {children}
    </ThinkingVisibilityContext.Provider>
  );
}

export function useThinkingVisibility(): ThinkingVisibilityContextType {
  const context = useContext(ThinkingVisibilityContext);
  if (context === undefined) {
    throw new Error('useThinkingVisibility must be used within a ThinkingVisibilityProvider');
  }
  return context;
}

export { ThinkingVisibilityContext };
